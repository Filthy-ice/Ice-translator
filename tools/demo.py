#!/usr/bin/env python3
"""Create an isolated, explicitly simulated SDD lifecycle for hands-on inspection."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools'))
from sdd import init, write


def main():
    base = Path(tempfile.mkdtemp(prefix='inshocking-sdd-demo-'))
    store, code = base / 'spec-store', base / 'demo-code'
    init(store)
    code.mkdir()
    (code / 'app.py').write_text('def add(a, b):\n    return a + b\n')
    (code / 'test_app.py').write_text('from app import add\nassert add(1, 2) == 3\nassert add(-2, 2) == 0\n')
    subprocess.run(['git', 'init', '-b', 'main', str(code)], check=True, capture_output=True)
    subprocess.run(['git', '-C', str(code), 'add', '.'], check=True)
    subprocess.run(['git', '-C', str(code), '-c', 'user.name=SDD Demo', '-c', 'user.email=demo@example.com',
                    'commit', '-qm', 'isolated simulated example'], check=True)

    def run(*args):
        r = subprocess.run([sys.executable, '-B', str(ROOT / 'sdd'), '--root', str(store), *args],
                           capture_output=True, text=True, check=True)
        print(r.stdout.strip())
        return r.stdout.strip()

    run('repo', 'app', '--path', str(code), '--branch', 'main', '--role', '隔离演示代码')
    run('new', 'CHG-001', '--title', '演示样例：整数加法', '--repos', 'app')
    p = store / 'features/CHG-001'
    (p / 'spec.md').write_text('# 演示规格（非真实业务批准）\n\n### REQ-001: 整数加法\nWHEN 输入两个整数 THEN SHALL 返回其和。\n')
    run('transition', 'CHG-001', 'CLARIFIED')
    run('transition', 'CHG-001', 'SPEC_REVIEW')
    h = run('hash', 'CHG-001')
    run('approve', 'CHG-001', '--hash', h, '--by', 'simulated-demo-user', '--reference', '明确标记的隔离演示批准，非真实业务批准')
    (p / 'design.md').write_text('# 演示 Design\n\n## 加法\n实现 app.add，通过正数和负数抵消场景验证。\n')
    write(p / 'tasks.json', {'baseline': None, 'tasks': [
        {'id': 'T-001', 'title': '实现并验证加法', 'requirements': ['REQ-001'], 'repo': 'app', 'status': 'todo', 'code_refs': []}]})
    write(p / 'traceability.json', {'baseline': None, 'design': [{'requirement': 'REQ-001', 'section': '## 加法'}],
        'verifications': [{'id': 'V-001', 'requirements': ['REQ-001'], 'repo': 'app', 'kind': 'VERIFIED_CODE',
                           'environment': 'local', 'required': True, 'command': [sys.executable, '-B', 'test_app.py']}]})
    run('plan-stamp', 'CHG-001')
    run('seal-plan', 'CHG-001')
    run('transition', 'CHG-001', 'IMPLEMENTING')
    run('task-done', 'CHG-001', 'T-001', '--code', 'app.py', 'test_app.py')
    run('check', 'CHG-001', 'V-001')
    run('transition', 'CHG-001', 'VERIFIED')
    run('accept', 'CHG-001', '--hash', h, '--by', 'simulated-demo-user', '--reference', '明确标记的隔离演示验收，非真实业务验收')
    run('index')
    run('validate')
    print('\n演示目录（可查看后自行删除）：' + str(base))
    print('全部批准与验收为模拟样例；没有修改实际 SDD 工程或业务仓库。')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
