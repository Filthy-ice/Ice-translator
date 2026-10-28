//! 超长文本的分段并行请求（FR-1.9，2026-10-28 用户要求"可以同时调用多次接口并行翻译后拼接"）。
//!
//! 串行时总耗时 = 段数 × 单段往返：12,000 字符按机翻的 450 字上限要 27 段，一段一秒多就是半分钟。
//! 段与段之间没有依赖（占位符编号在切块前就整体算好了），所以可以同时在飞。
//!
//! 并行之后有三条语义必须原样保住，这个模块存在的全部理由就是它们：
//! 1. **拼接按段序，不按完成先后**。并发请求谁先答完完全不固定，按完成顺序收会拼出一篇乱序译文；
//! 2. **任一段失败 ⇒ 整体失败**，并且立刻取消其余在飞请求——宁可一句都不给，不能给半篇，
//!    也不能在已经失败之后继续烧额度；
//! 3. **并发有上限**。公共机翻端点有免费额度，AI 服务有 429，一口气砸 27 个连接是自己找限流。

use std::collections::VecDeque;
use std::future::Future;
use std::sync::Arc;

use tokio::task::JoinSet;

use crate::error::{ProviderError, ProviderResult};

/// 同时在飞的分段数上限。
///
/// 4 是"接近线性提速"与"不被服务端掐"之间的折中：再高，单段延迟的抖动开始互相挤占，
/// 而公共端点的并发限制通常就在这附近。
pub(crate) const MAX_IN_FLIGHT: usize = 4;

/// 从待办队列取一段投进在飞窗口；队列空了什么都不做。
fn spawn_one<T, Fut>(
    set: &mut JoinSet<(usize, ProviderResult<String>)>,
    pending: &mut VecDeque<(usize, T)>,
    run: &Arc<dyn Fn(T) -> Fut + Send + Sync>,
) where
    T: Send + 'static,
    Fut: Future<Output = ProviderResult<String>> + Send + 'static,
{
    let Some((index, item)) = pending.pop_front() else {
        return;
    };
    let run = Arc::clone(run);
    set.spawn(async move { (index, run(item).await) });
}

/// 按 `items` 原序逐段执行 `run`，同时在飞不超过 `limit` 段。
///
/// 全部成功时返回与 `items` 同序的译文片段；任一段失败立刻取消其余在飞请求并返回该错误。
pub(crate) async fn map_segments<T, Fut>(
    items: Vec<T>,
    limit: usize,
    run: impl Fn(T) -> Fut + Send + Sync + 'static,
) -> ProviderResult<Vec<String>>
where
    T: Send + 'static,
    Fut: Future<Output = ProviderResult<String>> + Send + 'static,
{
    let mut pending: VecDeque<(usize, T)> = items.into_iter().enumerate().collect();
    let run: Arc<dyn Fn(T) -> Fut + Send + Sync> = Arc::new(run);
    let mut set = JoinSet::new();
    let mut slots: Vec<Option<String>> = vec![None; pending.len()];

    for _ in 0..limit.max(1) {
        spawn_one(&mut set, &mut pending, &run);
    }

    let mut failure = None;
    while let Some(done) = set.join_next().await {
        match done {
            Ok((index, Ok(text))) => {
                slots[index] = Some(text);
                // 空出一个槽位就补一段，窗口始终贴着 `limit`
                spawn_one(&mut set, &mut pending, &run);
            }
            Ok((_, Err(err))) => {
                failure = Some(err);
                break;
            }
            Err(join) => {
                failure = Some(ProviderError::Http(format!("分段请求异常终止：{join}")));
                break;
            }
        }
    }

    match failure {
        Some(err) => {
            set.abort_all();
            // `abort_all` 只发取消信号；把在飞的收干再返回，确保失败之后没有请求还在烧额度
            while set.join_next().await.is_some() {}
            Err(err)
        }
        None => Ok(slots
            .into_iter()
            .map(|slot| slot.expect("成功路径上每段都被填过"))
            .collect()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    use super::*;

    /// 每段固定成功，把序号写成译文，便于断言收结果的顺序
    fn numbered(index: usize) -> ProviderResult<String> {
        Ok(format!("#{index}"))
    }

    #[tokio::test]
    async fn results_come_back_in_segment_order() {
        // 让"后发的先答完"：段序 0 等 40ms、段序 4 立刻返回。
        // 按完成顺序收会拼成 #4#3#2#1#0，按段序收才是 #0#1#2#3#4。
        let texts = map_segments(vec![0usize, 1, 2, 3, 4], 5, |index| async move {
            tokio::time::sleep(Duration::from_millis(40 - 8 * index as u64)).await;
            numbered(index)
        })
        .await
        .expect("应全部成功");
        assert_eq!(texts.join(""), "#0#1#2#3#4");
    }

    #[tokio::test]
    async fn more_than_the_window_never_runs_at_once() {
        let live = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let items: Vec<usize> = (0..12).collect();
        let (live_job, peak_job) = (live.clone(), peak.clone());
        let texts = map_segments(items, 3, move |index| {
            let (live_job, peak_job) = (live_job.clone(), peak_job.clone());
            async move {
                let now = live_job.fetch_add(1, Ordering::SeqCst) + 1;
                peak_job.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(5)).await;
                live_job.fetch_sub(1, Ordering::SeqCst);
                numbered(index)
            }
        })
        .await
        .unwrap();
        assert_eq!(texts.len(), 12);
        let observed = peak.load(Ordering::SeqCst);
        assert!(observed <= 3, "并发窗口被突破：同时在飞 {observed} 段");
        assert!(observed >= 2, "根本没并行，同时在飞 {observed} 段");
    }

    #[tokio::test]
    async fn one_failure_fails_whole_request_and_cancels_the_rest() {
        let started = Instant::now();
        let err = map_segments(vec![0usize, 1, 2, 3], 2, |index| async move {
            if index == 0 {
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
            if index == 3 {
                return Err(ProviderError::Empty);
            }
            numbered(index)
        })
        .await
        .expect_err("任一段失败必须整体失败");
        assert!(matches!(err, ProviderError::Empty), "{err:?}");
        // 失败之后不再等剩下的段：窗口里那两段被取消，不该各自再睡 30ms
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "{:?}",
            started.elapsed()
        );

        // 取消不是假的：还在排的队列一段都不发
        let sent = Arc::new(Mutex::new(Vec::new()));
        let sent_job = sent.clone();
        let err = map_segments(vec![0usize, 1, 2, 3, 4, 5], 1, move |index| {
            let sent_job = sent_job.clone();
            async move {
                sent_job.lock().unwrap().push(index);
                if index == 1 {
                    return Err(ProviderError::Empty);
                }
                numbered(index)
            }
        })
        .await
        .expect_err("第二段失败应整体失败");
        assert!(matches!(err, ProviderError::Empty));
        let got = sent.lock().unwrap().clone();
        assert_eq!(got, vec![0, 1], "失败后不得继续投递后续段");
    }

    #[tokio::test]
    async fn single_segment_needs_no_window() {
        let texts = map_segments(vec![7usize], 4, |index| async move { numbered(index) })
            .await
            .unwrap();
        assert_eq!(texts, vec!["#7".to_string()]);
    }
}
