#!/usr/bin/env python3
"""Scan staged spec/skill/document bytes, including secrets removed only from worktree."""
from pathlib import Path
import subprocess
import sys
sys.path.insert(0, str(Path(__file__).resolve().parent))
from sdd import SECRETS, SPEC_DIRS


def staged_findings(root):
    names = subprocess.run(['git', '-C', str(root), 'diff', '--cached', '--name-only', '--diff-filter=ACMR', '-z'],
                           capture_output=True, check=True).stdout.split(b'\0')
    findings = []
    for name in filter(None, names):
        path = name.decode('utf-8', errors='replace')
        if not (path.startswith(tuple(d + '/' for d in (*SPEC_DIRS, 'skills', 'docs', 'examples'))) or path in ('README.md', 'AGENTS.md')):
            continue
        blob = subprocess.run(['git', '-C', str(root), 'show', ':' + path], capture_output=True, check=True).stdout
        try:
            content = blob.decode('utf-8')
        except UnicodeDecodeError:
            findings.append(path + ': binary 文件需人工检查')
            continue
        for n, line in enumerate(content.splitlines(), 1):
            for rule, regex in SECRETS.items():
                if regex.search(line):
                    findings.append(f'{path}:{n}: {rule}')
    return findings


if __name__ == '__main__':
    root = Path(subprocess.run(['git', 'rev-parse', '--show-toplevel'], capture_output=True, text=True, check=True).stdout.strip())
    findings = staged_findings(root)
    if findings:
        print('\n'.join(findings), file=sys.stderr)
        raise SystemExit(2)
    print('暂存规格/技能/文档敏感信息基线扫描通过')
