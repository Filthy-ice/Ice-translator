"""Offline SDD ledger. Python 3.11+, standard library only."""
import argparse
import contextlib
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
from datetime import datetime, timezone

SPEC_DIRS = ('project', 'codebase', 'features', 'quick', 'incidents', 'templates')

STATES = ['INTAKE', 'CLARIFIED', 'SPEC_REVIEW', 'APPROVED', 'DESIGNED',
          'IMPLEMENTING', 'VERIFIED', 'ACCEPTED']
KINDS = ['VERIFIED_CODE', 'VERIFIED_RUNTIME', 'BUSINESS_CONFIRMED', 'HYPOTHESIS']
ENVIRONMENTS = ['static', 'local', 'mock', 'sandbox', 'production', 'business']
RISK_FLAGS = ['message', 'rpc', 'business-state', 'redis', 'database', 'config',
              'permission', 'personal-data', 'external-write', 'cross-repo']
ID = re.compile(r'^[A-Z][A-Z0-9]*-\d{3,}$')
REQ = re.compile(r'^### (REQ-\d{3,}):\s+(.+)$', re.M)
SECRETS = {
    'private-key': re.compile(r'-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----'),
    'api-token': re.compile(r'\b(?:sk-[A-Za-z0-9_-]{20,}|gh[pousr]_[A-Za-z0-9]{30,}|AKIA[A-Z0-9]{16})\b'),
    'credential': re.compile(r'(?i)\b(?:password|api[_-]?key|secret|access[_-]?token)\s*[=:]\s*["\']?(?!\[REDACTED\]|<|\$)[A-Za-z0-9+/_.-]{12,}'),
    'private-ip': re.compile(r'\b(?:10\.\d{1,3}\.\d{1,3}\.\d{1,3}|192\.168\.\d{1,3}\.\d{1,3}|172\.(?:1[6-9]|2\d|3[01])\.\d{1,3}\.\d{1,3})\b'),
    'email': re.compile(r'\b[A-Za-z0-9._%+-]+@(?!example\.(?:com|org|net)\b)[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b'),
    'phone-cn': re.compile(r'(?<!\d)1[3-9]\d{9}(?!\d)'),
    'internal-host': re.compile(r'https?://[^\s/]+\.(?:internal|local|corp)(?:[/\s]|$)'),
}


def now():
    return datetime.now(timezone.utc).isoformat(timespec='seconds')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(',', ':')).encode()


def read(path):
    return json.loads(path.read_text())


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile('w', dir=path.parent, delete=False, encoding='utf-8') as f:
        json.dump(value, f, ensure_ascii=False, indent=2)
        f.write('\n')
    os.replace(f.name, path)


def git(path, *args):
    r = subprocess.run(['git', '-C', str(path), *args], capture_output=True, timeout=30)
    require(r.returncode == 0, 'Git 检查失败（请确认仓库、分支及至少一个提交）')
    return r.stdout


def root_for(value):
    if value:
        root = Path(value).expanduser().resolve()
    else:
        root = next((p for p in [Path.cwd(), *Path.cwd().parents]
                     if (p / 'project/config.json').is_file()), None)
        require(root is not None, '请在 SDD 工程运行，或传 --root PATH')
    require((root / 'project/config.json').is_file(), '工作区未初始化')
    return root


@contextlib.contextmanager
def locked(root):
    # One writer per workspace; never silently remove another writer's lock.
    path = root / '.sdd/write.lock'
    path.parent.mkdir(exist_ok=True)
    try:
        fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    except FileExistsError:
        raise ValueError('已有写操作或遗留 .sdd/write.lock；确认没有进程运行后再移除')
    try:
        os.write(fd, str(os.getpid()).encode())
        os.close(fd)
        yield
    finally:
        path.unlink()


def feature(root, change_id):
    require(bool(ID.fullmatch(change_id)), 'CHANGE-ID 格式如 CHG-001')
    for kind in ('features', 'quick'):
        p = root / kind / change_id
        if p.is_dir():
            require(p.resolve().is_relative_to(root / kind), '变更路径越界')
            return p, read(p / 'change.json')
    raise ValueError('变更不存在：' + change_id)


def requirements(p):
    matches = REQ.findall((p / 'spec.md').read_text())
    ids = [m[0] for m in matches]
    require(ids and len(set(ids)) == len(ids), 'Spec 需要唯一的 ### REQ-001: 标题')
    require(not re.search(r'\b(?:TODO|TBD)\b|待填写', (p / 'spec.md').read_text()), 'Spec 仍有占位内容')
    return set(ids)


def stamp(m):
    return f"Change: {m['id']}\nVersion: {m['version']}\nSpec-Hash: {(m.get('approval') or {}).get('hash', 'UNAPPROVED')}\n\n"


def baseline(m):
    return {'change': m['id'], 'version': m['version'], 'spec_hash': m['approval']['hash']}


def context_hash(m):
    return digest(canonical({k: m.get(k) for k in ('repos', 'questions', 'kind', 'risk_assessment')}))


def approved(p, m):
    a = m.get('approval')
    require(a and a['version'] == m['version'], '缺少当前版本批准')
    require(digest((p / 'spec.md').read_bytes()) == a['hash'], '批准 Spec 已漂移，先 deviation/revise 并重新评审')
    require(digest((p / a['snapshot']).read_bytes()) == a['hash'], '批准快照已漂移')
    require(a['context_hash'] == context_hash(m), '批准仓库/问题/风险范围已漂移，需重新评审')
    require(not any(q.get('blocking', True) and not q.get('resolution') for q in m['questions']), '存在阻塞问题')
    for q in m['questions']:
        require(not q.get('resolution') or q.get('source'), '已关闭问题必须记录确认/事实来源')
    if m['kind'] == 'quick':
        require(set(m.get('risk_assessment', {})) == set(RISK_FLAGS) and
                all(v is False for v in m['risk_assessment'].values()), 'Quick 必须逐项排除风险；否则升级 Feature')


def plan_hash(p):
    tasks = read(p / 'tasks.json')
    tasks = {**tasks, 'tasks': [{k: v for k, v in t.items() if k not in ('status', 'code_refs')}
                              for t in tasks['tasks']]}
    return digest((p / 'design.md').read_bytes() + canonical(tasks) + canonical(read(p / 'traceability.json')))


def mapped_repo(root, repo_id):
    reg = read(root / 'codebase/repositories.json')
    paths = read(root / '.sdd/repos.local.json') if (root / '.sdd/repos.local.json').exists() else {}
    require(repo_id in reg and repo_id in paths, f'仓库 {repo_id} 未登记/未绑定本机路径')
    path = Path(paths[repo_id]).expanduser().resolve()
    require(git(path, 'rev-parse', '--show-toplevel').decode().strip() == str(path), '绑定路径必须是 Git 仓库根目录')
    require(git(path, 'branch', '--show-current').decode().strip() == reg[repo_id]['branch'],
            f'{repo_id} 当前分支不符合登记分支')
    return path


def fingerprint(path):
    # Hash content, including untracked but not ignored files; commits/dirty state stay separate.
    files = git(path, 'ls-files', '-z', '--cached', '--others', '--exclude-standard').split(b'\0')
    entries = []
    for name in sorted(set(n for n in files if n)):
        p = path / os.fsdecode(name)
        if p.is_symlink():
            content = b'link:' + os.readlink(p).encode()
        elif p.is_file():
            content = p.read_bytes()
        else:
            content = b'MISSING'
        entries.append([os.fsdecode(name), digest(content), p.stat().st_mode & 0o111 if p.exists() else 0])
    return {'head': git(path, 'rev-parse', 'HEAD').decode().strip(), 'tree_hash': digest(canonical(entries)),
            'dirty': bool(git(path, 'status', '--porcelain'))}


def plan_valid(root, p, m, sealed=True):
    approved(p, m)
    reqs = requirements(p)
    require((p / 'design.md').read_text().startswith(stamp(m)), 'Design 基线不一致，使用 plan-stamp')
    require(not re.search(r'\b(?:TODO|TBD)\b|待填写', (p / 'design.md').read_text()), 'Design 仍有占位内容')
    tasks, trace = read(p / 'tasks.json'), read(p / 'traceability.json')
    require(tasks['baseline'] == baseline(m) == trace['baseline'], '跨文件版本/哈希不一致')
    for collection, key in ((tasks['tasks'], 'id'), (trace['verifications'], 'id')):
        ids = [item[key] for item in collection]
        require(ids and len(set(ids)) == len(ids), 'Task/Verification ID 为空或重复')
    covered_tasks, covered_design, covered_tests = set(), set(), set()
    for t in tasks['tasks']:
        require(t['requirements'] and set(t['requirements']) <= reqs, 'Task 无 Requirement 或引用越界')
        require(t['repo'] in m['repos'], 'Task 仓库超出变更范围')
        require(t['status'] in ('todo', 'doing', 'done'), 'Task 状态非法')
        require(t.get('title') and not re.search(r'\b(?:TODO|TBD)\b|待填写', t['title']), 'Task 标题缺失或有占位内容')
        covered_tasks.update(t['requirements'])
    design = (p / 'design.md').read_text()
    for row in trace['design']:
        require(row['requirement'] in reqs and row['section'] in design, 'Design 引用无法定位')
        covered_design.add(row['requirement'])
    for v in trace['verifications']:
        require(v['requirements'] and set(v['requirements']) <= reqs, 'Verification Requirement 引用越界')
        require(v['repo'] in m['repos'], 'Verification 仓库超出范围')
        require(v['kind'] in KINDS and v['environment'] in ENVIRONMENTS, '证据分类非法')
        require((v['kind'] != 'VERIFIED_RUNTIME' or v['environment'] in ('sandbox', 'production')) and
                (v['kind'] != 'BUSINESS_CONFIRMED' or v['environment'] == 'business') and
                (v['kind'] != 'VERIFIED_CODE' or v['environment'] in ('static', 'local', 'mock')),
                '证据标签与环境不匹配，Mock 不能记录为真实运行证据')
        require(type(v['required']) is bool, 'required 必须是布尔值')
        if v.get('command'):
            require(v['kind'] == 'VERIFIED_CODE' and v['environment'] in ('local', 'mock'), 'check 只运行本地检查；真实环境仅导入已授权证据')
            require(isinstance(v['command'], list) and all(isinstance(a, str) and a for a in v['command']), 'command 必须是非空 argv 字符串数组')
        if v['required']:
            require(v['kind'] != 'HYPOTHESIS', '假设不能作为必需验证')
            covered_tests.update(v['requirements'])
    require(reqs == covered_tasks == covered_design == covered_tests, 'Requirement 必须覆盖 Design、Task 和必需 Verification')
    for repo_id in m['repos']:
        require(repo_id in read(root / 'codebase/repositories.json'), '变更仓库未登记')
    if m['status'] != 'ACCEPTED':
        require(m['approval']['branches'] == {r: read(root / 'codebase/repositories.json')[r]['branch']
                                              for r in m['repos']}, '登记目标分支与批准范围不一致')
    if sealed:
        require(m.get('plan_hash') == plan_hash(p), '计划内容漂移，请 replan 后重新封存')


def verified(root, p, m, offline=False):
    plan_valid(root, p, m)
    tasks = read(p / 'tasks.json')['tasks']
    require(all(t['status'] == 'done' and t.get('code_refs') for t in tasks), '有未完成或缺代码/测试引用的 Task')
    for t in tasks:
        if offline:
            continue
        repo = mapped_repo(root, t['repo'])
        for ref in t['code_refs']:
            file = (repo / ref).resolve()
            require(file.is_relative_to(repo) and file.is_file(), 'Code/Test 引用失效')
    events = read(p / 'evidence.json')['events']
    for v in read(p / 'traceability.json')['verifications']:
        if not v['required']:
            continue
        relevant = [e for e in events if e['verification'] == v['id'] and e['baseline'] == baseline(m)
                    and e['plan_hash'] == m['plan_hash']]
        require(relevant and relevant[-1]['result'] == 'passed', f"{v['id']} 缺少当前基线成功证据或最后一次失败")
        e = relevant[-1]
        if not offline:
            require(e['repo_state']['tree_hash'] == fingerprint(mapped_repo(root, v['repo']))['tree_hash'],
                    f"{v['id']} 检查后代码改变，证据过期")
        require(e['requirements'] == v['requirements'] and e['repo'] == v['repo'] and
                e['kind'] == v['kind'] and e['environment'] == v['environment'], '证据元数据与验证计划不一致')
        artifact = p / e['artifact']
        require(artifact.resolve().is_relative_to(p.resolve()) and artifact.is_file() and
                digest(artifact.read_bytes()) == e['artifact_hash'], '证据附件缺失或漂移')


def audit(m, action, detail):
    m['history'].append({'at': now(), 'action': action, 'detail': detail})


def views(p, m):
    tasks = read(p / 'tasks.json')['tasks']
    lines = [stamp(m) + '# Tasks（由 tasks.json 生成，勿直接修改）\n']
    for t in tasks:
        lines.append(f"- [{'x' if t['status'] == 'done' else ' '}] {t['id']} {t['title']} | {t['repo']} | {', '.join(t['requirements'])}\n")
        if t.get('code_refs'):
            lines.append(f"  Code/Test: {', '.join(t['code_refs'])}\n")
    task_text = ''.join(lines)
    lines = [stamp(m) + '# Evidence（由 evidence.json 生成，勿直接修改）\n']
    for e in read(p / 'evidence.json')['events']:
        lines.append(f"\n- {e['at']} {e['verification']} **{e['result']}** {e['kind']}/{e['environment']}\n"
                     f"  {e['summary']}；附件：{e['artifact']}；Spec：{e['baseline']['spec_hash']}\n")
    return {'tasks.md': task_text, 'evidence.md': ''.join(lines)}


def render(p, m):
    for name, content in views(p, m).items():
        (p / name).write_text(content)


def scan(root):
    findings = []
    for folder in (*SPEC_DIRS, 'skills', 'docs', 'examples'):
        for p in sorted((root / folder).rglob('*')):
            if not p.is_file() or p.is_symlink() or p.name == '.DS_Store':
                continue
            try:
                content = p.read_text()
            except UnicodeDecodeError:
                findings.append(f'{p.relative_to(root)}: binary 文件需人工检查')
                continue
            for n, line in enumerate(content.splitlines(), 1):
                for name, regex in SECRETS.items():
                    if regex.search(line):
                        # Only location/rule; never print the sensitive value.
                        findings.append(f'{p.relative_to(root)}:{n}: {name}')
    return findings


def init(root):
    require(not (root / 'project/config.json').exists(), '工作区已存在，不覆盖')
    source = Path(__file__).resolve().parents[1]
    for d in SPEC_DIRS:
        (root / d).mkdir(parents=True, exist_ok=True)
    if root != source:
        import shutil
        shutil.copytree(source / 'templates', root / 'templates', dirs_exist_ok=True)
    write(root / 'project/config.json', {'schema_version': 1, 'name': 'inShocking SDD', 'workflow_version': '1.0.0'})
    write(root / 'codebase/repositories.json', {})
    (root / 'project/CONSTITUTION.md').write_text('# 项目边界\n\n已批准行为可独立完成；新增业务判断由人决定；外部副作用需明确授权。\n\n功能局部方案留在 Feature。事实使用四级标签，Mock 不证明真实环境。\n')
    (root / 'codebase/ARCHITECTURE.md').write_text('# 代码地图\n\n登记实际仓库后记录责任、入口、状态流、集成、测试基线和风险；每项事实附代码或运行来源。\n')
    (root / 'project/ROADMAP.md').write_text('# 路线图\n\n待接入第一个真实项目；不根据历史需求推断新项目事实。\n')


def execute(root, a):
    if a.cmd == 'repo':
        require(re.fullmatch(r'[a-z][a-z0-9-]{0,63}', a.name), 'repo 名称使用小写字母、数字、连字符')
        path = Path(a.path).expanduser().resolve()
        require(git(path, 'rev-parse', '--show-toplevel').decode().strip() == str(path), '必须登记仓库根目录')
        regpath = root / 'codebase/repositories.json'
        reg = read(regpath)
        reg[a.name] = {'branch': a.branch, 'role': a.role}
        write(regpath, reg)
        localpath = root / '.sdd/repos.local.json'
        local = read(localpath) if localpath.exists() else {}
        local[a.name] = str(path)
        write(localpath, local)
        print('仓库已绑定；未修改业务仓库')
        return
    if a.cmd == 'new':
        require(ID.fullmatch(a.id), 'ID 格式如 CHG-001')
        require(re.fullmatch(r'v\d+\.\d+(?:\.\d+)?', a.version), '版本格式如 v0.1')
        require(not any((root / k / a.id).exists() for k in ('features', 'quick')), 'ID 已存在')
        require(a.repos and len(set(a.repos)) == len(a.repos), '仓库范围为空或重复')
        reg = read(root / 'codebase/repositories.json')
        require(set(a.repos) <= set(reg), '先 repo 登记实际代码仓库')
        if a.quick:
            require(len(a.repos) == 1, 'Quick 只适用于单仓')
        p = root / ('quick' if a.quick else 'features') / a.id
        p.mkdir()
        m = {'schema_version': 1, 'id': a.id, 'title': a.title, 'version': a.version,
             'status': 'INTAKE', 'kind': 'quick' if a.quick else 'feature', 'repos': a.repos,
             'questions': [], 'history': [], 'approval': None, 'plan_hash': None}
        if a.quick:
            m['risk_assessment'] = {flag: None for flag in RISK_FLAGS}
        audit(m, 'new', '本地草稿')
        write(p / 'change.json', m)
        for name in ('spec.md', 'design.md'):
            content = (root / 'templates' / name).read_text().replace('{{ID}}', a.id).replace('{{TITLE}}', a.title)
            (p / name).write_text(content)
        for name in ('tasks.json', 'traceability.json', 'evidence.json'):
            value = read(root / 'templates' / name)
            value = json.loads(json.dumps(value, ensure_ascii=False).replace('{{REPO}}', a.repos[0]))
            write(p / name, value)
        render(p, m)
        print(p)
        return
    if a.cmd == 'scan':
        findings = scan(root)
        require(not findings, '\n'.join(findings))
        print('敏感信息基线扫描通过（不能替代完整 DLP/Gitleaks）')
        return
    if a.cmd == 'index':
        rows = ['# STATE（由 change.json 生成）\n\n| Change | Version | Status | Title |\n|---|---|---|---|\n']
        for kind in ('features', 'quick'):
            for file in sorted((root / kind).glob('*/change.json')):
                m = read(file)
                rows.append(f"| [{m['id']}](../{kind}/{m['id']}/spec.md) | {m['version']} | {m['status']} | {m['title']} |\n")
        (root / 'project/STATE.md').write_text(''.join(rows))
        print('导航索引已刷新；单项以 change.json 为准')
        return
    if a.cmd == 'validate' and not a.id:
        for kind in ('features', 'quick'):
            for file in (root / kind).glob('*/change.json'):
                validate(root, file.parent, read(file), offline=a.offline)
        require(not scan(root), '\n'.join(scan(root)))
        print('所有变更及敏感信息基线扫描通过')
        return
    p, m = feature(root, a.id)
    if a.cmd == 'status':
        print(json.dumps(m, ensure_ascii=False, indent=2))
        for repo_id in m['repos']:
            try:
                state = fingerprint(mapped_repo(root, repo_id))
                print(repo_id, json.dumps(state))
            except ValueError as e:
                print(repo_id, str(e))
        return
    if a.cmd == 'validate':
        validate(root, p, m, offline=a.offline)
        print('变更门禁通过：' + m['status'])
        return
    if a.cmd == 'hash':
        print(digest((p / 'spec.md').read_bytes()))
        return
    if a.cmd == 'refresh':
        render(p, m)
        print('生成视图已刷新；规格、批准和实施进度未修改')
        return
    if a.cmd == 'approve':
        require(m['status'] == 'SPEC_REVIEW', '只能批准 SPEC_REVIEW')
        requirements(p)
        require(not any(q.get('blocking', True) and not q.get('resolution') for q in m['questions']), '存在阻塞问题')
        require(all(not q.get('resolution') or q.get('source') for q in m['questions']), '已关闭问题缺确认来源')
        require(not scan(root), '\n'.join(scan(root)))
        actual = digest((p / 'spec.md').read_bytes())
        require(a.hash == actual, '提供的批准哈希与当前 Spec 不一致')
        require(a.by.strip() and a.reference.strip(), '批准必须有决定人和原始确认出处')
        if m['kind'] == 'quick':
            require(set(m['risk_assessment']) == set(RISK_FLAGS) and
                    all(v is False for v in m['risk_assessment'].values()), 'Quick 风险评估未完成；风险不明确则使用 Feature')
        snapshot = f"approvals/{m['version']}-{actual}.md"
        require(not (p / snapshot).exists(), '该规格已保存快照，需新版本')
        (p / 'approvals').mkdir(exist_ok=True)
        (p / snapshot).write_bytes((p / 'spec.md').read_bytes())
        m['approval'] = {'hash': actual, 'version': m['version'], 'by': a.by, 'reference': a.reference,
                         'at': now(), 'snapshot': snapshot, 'scope': 'local-delivery',
                         'context_hash': context_hash(m),
                         'branches': {r: read(root / 'codebase/repositories.json')[r]['branch'] for r in m['repos']}}
        m['status'] = 'APPROVED'
        audit(m, 'approve', m['approval'])
    elif a.cmd == 'plan-stamp':
        require(m['status'] == 'APPROVED', 'plan-stamp 只在 APPROVED 使用')
        approved(p, m)
        content = (p / 'design.md').read_text()
        content = re.sub(r'\AChange: .*\nVersion: .*\nSpec-Hash: .*\n\n', '', content)
        (p / 'design.md').write_text(stamp(m) + content)
        for file in ('tasks.json', 'traceability.json'):
            value = read(p / file)
            value['baseline'] = baseline(m)
            write(p / file, value)
        audit(m, 'plan-stamp', baseline(m))
    elif a.cmd == 'seal-plan':
        require(m['status'] == 'APPROVED', '先批准，或 replan')
        require(not scan(root), '\n'.join(scan(root)))
        plan_valid(root, p, m, sealed=False)
        m['plan_hash'] = plan_hash(p)
        m['status'] = 'DESIGNED'
        audit(m, 'seal-plan', m['plan_hash'])
    elif a.cmd == 'transition':
        allowed = {'INTAKE': 'CLARIFIED', 'CLARIFIED': 'SPEC_REVIEW',
                   'DESIGNED': 'IMPLEMENTING', 'IMPLEMENTING': 'VERIFIED'}
        require(allowed.get(m['status']) == a.to, f"不允许 {m['status']} → {a.to}；批准/验收/退回用专用命令")
        if a.to == 'SPEC_REVIEW':
            requirements(p)
        if a.to == 'IMPLEMENTING':
            plan_valid(root, p, m)
            for r in m['repos']:
                mapped_repo(root, r)
        if a.to == 'VERIFIED':
            verified(root, p, m)
            require(not scan(root), '\n'.join(scan(root)))
        m['status'] = a.to
        audit(m, 'transition', a.to)
    elif a.cmd in ('deviation', 'revise'):
        require(a.reason.strip(), '必须记录原因')
        if a.cmd == 'revise':
            require(re.fullmatch(r'v\d+\.\d+(?:\.\d+)?', a.version), '版本格式如 v0.2')
            require(tuple(map(int, a.version[1:].split('.'))) > tuple(map(int, m['version'][1:].split('.'))), '新版本必须递增')
            m['version'] = a.version
        m['status'] = 'SPEC_REVIEW'
        m['approval'] = None
        m['plan_hash'] = None
        m.pop('acceptance', None)
        audit(m, 'SPEC_DEVIATION' if a.cmd == 'deviation' else 'revise', a.reason)
    elif a.cmd == 'replan':
        require(m['status'] in ('DESIGNED', 'IMPLEMENTING', 'VERIFIED'), '当前状态不支持 replan')
        approved(p, m)
        m['status'], m['plan_hash'] = 'APPROVED', None
        audit(m, 'replan', a.reason)
    elif a.cmd == 'task-done':
        require(m['status'] == 'IMPLEMENTING', 'Task 只能在 IMPLEMENTING 完成')
        plan_valid(root, p, m)
        doc = read(p / 'tasks.json')
        task = next((t for t in doc['tasks'] if t['id'] == a.task), None)
        require(task is not None, 'Task 不存在')
        repo = mapped_repo(root, task['repo'])
        for ref in a.code:
            file = (repo / ref).resolve()
            require(file.is_relative_to(repo) and file.is_file(), 'Code/Test 引用需是仓库内实际文件')
        task['status'], task['code_refs'] = 'done', a.code
        write(p / 'tasks.json', doc)
        audit(m, 'task-done', a.task)
    elif a.cmd in ('check', 'record'):
        require(m['status'] == 'IMPLEMENTING', '证据在 IMPLEMENTING 记录；代码变动用 replan 重新进入实施')
        plan_valid(root, p, m)
        v = next((v for v in read(p / 'traceability.json')['verifications'] if v['id'] == a.verification), None)
        require(v is not None, 'Verification 不存在')
        repo = mapped_repo(root, v['repo'])
        state = fingerprint(repo)
        if a.cmd == 'check':
            require(v.get('command'), '未登记本地检查命令')
            try:
                r = subprocess.run(v['command'], cwd=repo, capture_output=True, timeout=a.timeout)
                output = r.stdout + b'\n' + r.stderr
                result = 'passed' if r.returncode == 0 else 'failed'
                summary = f"argv={json.dumps(v['command'], ensure_ascii=False)} exit={r.returncode}"
            except subprocess.TimeoutExpired as e:
                output = (e.stdout or b'') + b'\n' + (e.stderr or b'')
                result, summary = 'failed', '检查超时'
            require(fingerprint(repo)['tree_hash'] == state['tree_hash'], '检查修改了源码/非忽略文件；清理构建输出后重新运行')
        else:
            require(not v.get('command'), '有 command 的验证必须通过 check 运行，不能手工声称通过')
            source = Path(a.source).expanduser()
            require(source.is_file(), '证据来源文件不存在')
            output, result, summary = source.read_bytes(), a.result, a.summary
        text_output = output.decode('utf-8', errors='replace')
        for regex in SECRETS.values():
            text_output = regex.sub('[REDACTED]', text_output)
        artifact = f'evidence/{len(read(p / "evidence.json")["events"]) + 1:04d}-{v["id"]}.txt'
        (p / 'evidence').mkdir(exist_ok=True)
        (p / artifact).write_text(text_output)
        event = {'at': now(), 'verification': v['id'], 'requirements': v['requirements'],
                 'repo': v['repo'], 'repo_state': state, 'kind': v['kind'], 'environment': v['environment'],
                 'baseline': baseline(m), 'plan_hash': m['plan_hash'], 'result': result,
                 'summary': summary, 'artifact': artifact, 'artifact_hash': digest((p / artifact).read_bytes())}
        doc = read(p / 'evidence.json')
        doc['events'].append(event)
        write(p / 'evidence.json', doc)
        audit(m, a.cmd, {'verification': v['id'], 'result': result})
        write(p / 'change.json', m)
        render(p, m)
        print(f"{v['id']}: {result}（输出已脱敏）")
        return 0 if result == 'passed' else 1
    elif a.cmd == 'accept':
        require(m['status'] == 'VERIFIED', '先完成 VERIFIED')
        verified(root, p, m)
        require(not scan(root), '\n'.join(scan(root)))
        require(a.hash == m['approval']['hash'], '验收必须引用批准哈希')
        require(a.by.strip() and a.reference.strip(), '验收必须有实际验收人和出处')
        m['acceptance'] = {'at': now(), 'by': a.by, 'reference': a.reference, 'baseline': baseline(m)}
        m['status'] = 'ACCEPTED'
        audit(m, 'accept', m['acceptance'])
    else:
        raise ValueError('未知命令')
    write(p / 'change.json', m)
    render(p, m)
    print(m['id'], m['version'], m['status'])


def validate(root, p, m, offline=False):
    require(m['schema_version'] == 1 and m['status'] in STATES, '未知结构/状态')
    require(m['id'] == p.name, 'CHANGE-ID 与目录不一致')
    for file in ('spec.md', 'design.md', 'tasks.json', 'tasks.md', 'traceability.json', 'evidence.json', 'evidence.md'):
        require((p / file).is_file(), '缺少 ' + file)
    if STATES.index(m['status']) >= STATES.index('SPEC_REVIEW'):
        requirements(p)
    if STATES.index(m['status']) >= STATES.index('APPROVED'):
        approved(p, m)
    if STATES.index(m['status']) >= STATES.index('DESIGNED'):
        plan_valid(root, p, m)
    if m['status'] in ('VERIFIED', 'ACCEPTED'):
        verified(root, p, m, offline=offline or m['status'] == 'ACCEPTED')
    if m['status'] == 'ACCEPTED':
        require(m.get('acceptance', {}).get('baseline') == baseline(m), '验收基线不一致')
    for name in ('tasks.md', 'evidence.md'):
        require((p / name).read_text().startswith(stamp(m)), name + ' 版本/哈希漂移')
        require((p / name).read_text() == views(p, m)[name], name + ' 生成视图与 JSON 不一致，使用 refresh')


def parser():
    p = argparse.ArgumentParser(description='inShocking 本地 SDD：规格、批准、追踪与证据')
    p.add_argument('--root', help='SDD 工程路径，默认从当前目录向上查找')
    sub = p.add_subparsers(dest='cmd', required=True)
    c = sub.add_parser('init'); c.add_argument('path')
    c = sub.add_parser('repo'); c.add_argument('name'); c.add_argument('--path', required=True); c.add_argument('--branch', required=True); c.add_argument('--role', required=True)
    c = sub.add_parser('new'); c.add_argument('id'); c.add_argument('--title', required=True); c.add_argument('--repos', nargs='+', required=True); c.add_argument('--version', default='v0.1'); c.add_argument('--quick', action='store_true')
    for cmd in ('status', 'hash', 'plan-stamp', 'seal-plan', 'refresh'):
        c = sub.add_parser(cmd); c.add_argument('id')
    c = sub.add_parser('validate'); c.add_argument('id', nargs='?'); c.add_argument('--offline', action='store_true', help='仅查存档，不校验本机业务源码；适用于 CI 和迁移')
    sub.add_parser('scan'); sub.add_parser('index')
    for cmd in ('approve', 'accept'):
        c = sub.add_parser(cmd); c.add_argument('id'); c.add_argument('--hash', required=True); c.add_argument('--by', required=True); c.add_argument('--reference', required=True)
    c = sub.add_parser('transition'); c.add_argument('id'); c.add_argument('to', choices=STATES)
    for cmd in ('deviation', 'revise', 'replan'):
        c = sub.add_parser(cmd); c.add_argument('id'); c.add_argument('--reason', required=True)
        if cmd == 'revise': c.add_argument('--version', required=True)
    c = sub.add_parser('task-done'); c.add_argument('id'); c.add_argument('task'); c.add_argument('--code', nargs='+', required=True)
    c = sub.add_parser('check'); c.add_argument('id'); c.add_argument('verification'); c.add_argument('--timeout', type=int, default=120)
    c = sub.add_parser('record'); c.add_argument('id'); c.add_argument('verification'); c.add_argument('--source', required=True); c.add_argument('--result', choices=['passed', 'failed', 'observed'], required=True); c.add_argument('--summary', required=True)
    return p


def main(argv=None):
    a = parser().parse_args(argv)
    try:
        if a.cmd == 'init':
            init(Path(a.path).expanduser().resolve())
            print('工作区已初始化')
            return 0
        root = root_for(a.root)
        if a.cmd in ('status', 'hash', 'validate', 'scan'):
            return execute(root, a) or 0
        with locked(root):
            return execute(root, a) or 0
    except (ValueError, KeyError, TypeError, OSError, subprocess.SubprocessError) as e:
        print('SDD BLOCKED: ' + str(e), file=sys.stderr)
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
