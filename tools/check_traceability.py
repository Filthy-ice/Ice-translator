#!/usr/bin/env python3
"""核对需求基线与追踪矩阵的编号集合是否一致（CHG-001 REQ-004 的必需验证）。"""
from pathlib import Path
import re
import sys

ROW = re.compile(r'^\|\s*((?:FR|NFR)-\d+(?:\.\d+)?)\b')


def ids(path):
    found = set()
    for line in path.read_text(encoding='utf-8').splitlines():
        m = ROW.match(line)
        if m:
            found.add(m.group(1))
    return found


def main(root):
    reqs = root / 'docs/01-requirements.md'
    matrix = root / 'docs/traceability.md'
    for f in (reqs, matrix):
        if not f.is_file():
            print(f'缺少文件：{f}', file=sys.stderr)
            return 2
    a, b = ids(reqs), ids(matrix)
    findings = [f'需求有、矩阵缺：{x}' for x in sorted(a - b)]
    findings += [f'矩阵有、需求无：{x}' for x in sorted(b - a)]
    if findings:
        print('\n'.join(findings), file=sys.stderr)
        print(f'需求 {len(a)} 条 / 矩阵 {len(b)} 行，不一致', file=sys.stderr)
        return 2
    print(f'需求与追踪矩阵编号一致：{len(a)} 条')
    return 0


if __name__ == '__main__':
    root = next((p for p in [Path.cwd(), *Path.cwd().parents]
                 if (p / 'project/config.json').is_file()), None)
    if root is None:
        print('请在 SDD 工程运行', file=sys.stderr)
        raise SystemExit(2)
    raise SystemExit(main(root))
