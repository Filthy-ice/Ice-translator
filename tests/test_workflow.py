import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools'))
import sdd
from install_skills import install
from check_staged import staged_findings


class Fixture:
    """夹具操作；子类用 setUp 决定 SDD 工程与业务仓库的布局。"""

    def g(self, *args):
        return subprocess.run(['git', '-C', str(self.repo), *args], check=True, capture_output=True).stdout

    def cli(self, *args, code=0):
        r = subprocess.run([sys.executable, str(ROOT / 'sdd'), '--root', str(self.root), *args], capture_output=True, text=True)
        self.assertEqual(r.returncode, code, r.stderr + r.stdout)
        return r

    def edit(self, file, fn):
        v = sdd.read(self.p / file)
        fn(v)
        sdd.write(self.p / file, v)

    def approve(self):
        self.cli('transition', 'CHG-001', 'CLARIFIED')
        self.cli('transition', 'CHG-001', 'SPEC_REVIEW')
        h = sdd.digest((self.p / 'spec.md').read_bytes())
        self.cli('approve', 'CHG-001', '--hash', h, '--by', 'fixture-user', '--reference', 'isolated-test-fixture')

    def plan(self, cmd=None, environment='local', kind='VERIFIED_CODE'):
        self.approve()
        (self.p / 'design.md').write_text('# Design\n\n## 加法实现\n复用 calc.add，并通过断言验证。\n')
        sdd.write(self.p / 'tasks.json', {'baseline': None, 'tasks': [
            {'id': 'T-001', 'title': '验证加法行为', 'requirements': ['REQ-001'], 'repo': 'app', 'status': 'todo', 'code_refs': []}]})
        v = {'id': 'V-001', 'requirements': ['REQ-001'], 'repo': 'app', 'kind': kind, 'environment': environment, 'required': True}
        if cmd is None and environment == 'local':
            cmd = [sys.executable, '-B', 'test_calc.py']
        if cmd:
            v['command'] = cmd
        sdd.write(self.p / 'traceability.json', {'baseline': None, 'design': [
            {'requirement': 'REQ-001', 'section': '## 加法实现'}], 'verifications': [v]})
        self.cli('plan-stamp', 'CHG-001')

    def implementing(self):
        self.plan()
        self.cli('seal-plan', 'CHG-001')
        self.cli('transition', 'CHG-001', 'IMPLEMENTING')

    def verify(self):
        self.implementing()
        self.cli('task-done', 'CHG-001', 'T-001', '--code', 'calc.py', 'test_calc.py')
        self.cli('check', 'CHG-001', 'V-001')
        self.cli('transition', 'CHG-001', 'VERIFIED')


class Workflow(Fixture, unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / 'spec-store'
        sdd.init(self.root)
        self.repo = Path(self.tmp.name) / 'code'
        self.repo.mkdir()
        self.g('init', '-b', 'main')
        (self.repo / 'calc.py').write_text('def add(a, b):\n    return a + b\n')
        (self.repo / 'test_calc.py').write_text('from calc import add\nassert add(1, 2) == 3\n')
        self.g('add', '.')
        self.g('-c', 'user.name=SDD Test', '-c', 'user.email=test@example.com', 'commit', '-qm', 'fixture')
        self.cli('repo', 'app', '--path', str(self.repo), '--branch', 'main', '--role', '测试样本')
        self.cli('new', 'CHG-001', '--title', '计算器验证样本', '--repos', 'app')
        self.p = self.root / 'features/CHG-001'
        (self.p / 'spec.md').write_text('# 测试规格\n\n### REQ-001: 两个整数相加\nWHEN 1 和 2 相加 THEN SHALL 返回 3。\n')

    def test_end_to_end_acceptance_is_separate(self):
        self.verify()
        h = self.cli('hash', 'CHG-001').stdout.strip()
        self.assertNotIn('acceptance', sdd.read(self.p / 'change.json'))
        self.cli('accept', 'CHG-001', '--hash', h, '--by', 'fixture-user', '--reference', 'isolated-test-acceptance')
        self.cli('validate', 'CHG-001')

    def test_skip_approval_blocked(self):
        self.cli('transition', 'CHG-001', 'IMPLEMENTING', code=2)
        self.cli('check', 'CHG-001', 'V-001', code=2)

    def test_wrong_approval_hash_blocked(self):
        self.cli('transition', 'CHG-001', 'CLARIFIED')
        self.cli('transition', 'CHG-001', 'SPEC_REVIEW')
        self.cli('approve', 'CHG-001', '--hash', '0' * 64, '--by', 'user', '--reference', 'fixture', code=2)

    def test_blocking_question(self):
        self.edit('change.json', lambda v: v['questions'].append({'id': 'Q-001', 'blocking': True, 'resolution': None}))
        self.cli('transition', 'CHG-001', 'CLARIFIED')
        self.cli('transition', 'CHG-001', 'SPEC_REVIEW')
        h = self.cli('hash', 'CHG-001').stdout.strip()
        self.cli('approve', 'CHG-001', '--hash', h, '--by', 'user', '--reference', 'fixture', code=2)

    def test_spec_drift(self):
        self.approve()
        with (self.p / 'spec.md').open('a') as f: f.write('新的业务规则。\n')
        self.cli('validate', 'CHG-001', code=2)
        self.cli('plan-stamp', 'CHG-001', code=2)

    def test_snapshot_drift(self):
        self.approve()
        a = sdd.read(self.p / 'change.json')['approval']
        (self.p / a['snapshot']).write_text('changed')
        self.cli('validate', 'CHG-001', code=2)

    def test_missing_requirement_coverage(self):
        self.plan()
        self.edit('traceability.json', lambda v: v['design'].clear())
        self.cli('seal-plan', 'CHG-001', code=2)

    def test_version_mismatch(self):
        self.plan()
        self.edit('tasks.json', lambda v: v['baseline'].update(version='v9.9'))
        self.cli('seal-plan', 'CHG-001', code=2)

    def test_mock_cannot_be_runtime_evidence(self):
        self.plan(environment='mock', kind='VERIFIED_RUNTIME', cmd=['true'])
        self.cli('seal-plan', 'CHG-001', code=2)

    def test_plan_drift(self):
        self.implementing()
        self.edit('traceability.json', lambda v: v['verifications'][0].update(command=['true']))
        self.cli('check', 'CHG-001', 'V-001', code=2)

    def test_manual_pass_cannot_replace_command(self):
        self.implementing()
        self.cli('record', 'CHG-001', 'V-001', '--source', str(self.repo / 'test_calc.py'), '--result', 'passed', '--summary', 'manual', code=2)

    def test_failed_check_blocks_verified(self):
        self.implementing()
        (self.repo / 'calc.py').write_text('def add(a, b):\n    return 0\n')
        self.cli('task-done', 'CHG-001', 'T-001', '--code', 'calc.py')
        self.cli('check', 'CHG-001', 'V-001', code=1)
        self.cli('transition', 'CHG-001', 'VERIFIED', code=2)

    def test_stale_evidence(self):
        self.verify()
        (self.repo / 'calc.py').write_text('def add(a, b):\n    return a - b\n')
        self.cli('validate', 'CHG-001', code=2)

    def test_untracked_code_invalidates_evidence(self):
        self.verify()
        (self.repo / 'new_logic.py').write_text('print(42)\n')
        self.cli('validate', 'CHG-001', code=2)

    def test_evidence_attachment_hash(self):
        self.verify()
        artifact = sdd.read(self.p / 'evidence.json')['events'][0]['artifact']
        (self.p / artifact).write_text('forged')
        self.cli('validate', 'CHG-001', code=2)

    def test_wrong_branch_blocked(self):
        self.implementing()
        self.g('switch', '-c', 'wrong')
        self.cli('check', 'CHG-001', 'V-001', code=2)

    def test_external_rejection_returns_review(self):
        self.verify()
        self.cli('deviation', 'CHG-001', '--reason', '测试样本：外部契约拒绝')
        m = sdd.read(self.p / 'change.json')
        self.assertEqual(m['status'], 'SPEC_REVIEW')
        self.assertIsNone(m['approval'])
        self.cli('transition', 'CHG-001', 'IMPLEMENTING', code=2)
        self.cli('revise', 'CHG-001', '--version', 'v0.2', '--reason', '新业务方案待评审')
        self.assertTrue(list((self.p / 'approvals').glob('*.md')))

    def test_path_traversal_rejected(self):
        self.cli('status', '../CHG-001', code=2)
        self.implementing()
        self.cli('task-done', 'CHG-001', 'T-001', '--code', '../spec-store/AGENTS.md', code=2)

    def test_scanner_does_not_expose_value(self):
        value = 'sk-' + 'a' * 30
        (self.root / 'project/private.md').write_text(value)
        r = self.cli('scan', code=2)
        self.assertNotIn(value, r.stderr)
        self.assertIn('api-token', r.stderr)

    def test_quick_requires_risk_assessment(self):
        self.cli('new', 'CHG-002', '--title', '轻量样本', '--repos', 'app', '--quick')
        p = self.root / 'quick/CHG-002'
        (p / 'spec.md').write_text('### REQ-001: 文案\nWHEN 查看 THEN SHALL 显示文案。\n')
        self.cli('transition', 'CHG-002', 'CLARIFIED')
        self.cli('transition', 'CHG-002', 'SPEC_REVIEW')
        h = self.cli('hash', 'CHG-002').stdout.strip()
        self.cli('approve', 'CHG-002', '--hash', h, '--by', 'user', '--reference', 'fixture', code=2)

    def test_lock_prevents_concurrent_writer(self):
        with sdd.locked(self.root):
            self.cli('transition', 'CHG-001', 'CLARIFIED', code=2)

    def test_closed_question_needs_source(self):
        self.edit('change.json', lambda v: v['questions'].append({'id': 'Q-001', 'blocking': True, 'resolution': '决定采用 A'}))
        self.cli('transition', 'CHG-001', 'CLARIFIED')
        self.cli('transition', 'CHG-001', 'SPEC_REVIEW')
        self.cli('approve', 'CHG-001', '--hash', self.cli('hash', 'CHG-001').stdout.strip(), '--by', 'user', '--reference', 'fixture', code=2)

    def test_scope_drift_after_approval(self):
        self.approve()
        self.edit('change.json', lambda v: v['repos'].append('other'))
        self.cli('plan-stamp', 'CHG-001', code=2)

    def test_accepted_is_historical_after_future_changes(self):
        self.test_end_to_end_acceptance_is_separate()
        (self.repo / 'calc.py').write_text('# next feature\n')
        self.cli('validate', 'CHG-001')

    def test_offline_validation_does_not_claim_code_freshness(self):
        self.verify()
        (self.repo / 'calc.py').write_text('# changed\n')
        self.cli('validate', 'CHG-001', '--offline')
        self.cli('accept', 'CHG-001', '--hash', self.cli('hash', 'CHG-001').stdout.strip(), '--by', 'user', '--reference', 'fixture', code=2)

    def test_generated_views_cannot_hide_task_state(self):
        self.implementing()
        with (self.p / 'tasks.md').open('a') as f: f.write('全部完成（错误编辑生成视图）\n')
        self.cli('validate', 'CHG-001', code=2)
        self.cli('refresh', 'CHG-001')
        self.cli('validate', 'CHG-001')

    def test_staged_secret_detected_even_when_worktree_is_cleaned(self):
        folder = self.repo / 'project'
        folder.mkdir()
        secret = folder / 'facts.md'
        secret.write_text('sk-' + 'b' * 30)
        self.g('add', 'project/facts.md')
        secret.write_text('clean working copy')
        findings = staged_findings(self.repo)
        self.assertTrue(any('api-token' in line for line in findings))
        self.assertNotIn('b' * 30, str(findings))

    def test_visible_layout_and_nested_root_discovery(self):
        self.assertFalse((self.root / '.specs').exists())
        self.assertTrue((self.root / 'project/config.json').is_file())
        r = subprocess.run([sys.executable, '-B', str(ROOT / 'sdd'), 'hash', 'CHG-001'],
                           cwd=self.p, capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.strip(), sdd.digest((self.p / 'spec.md').read_bytes()))

    def test_scanner_covers_all_visible_spec_categories(self):
        value = 'sk-' + 'c' * 30
        for category in sdd.SPEC_DIRS:
            with self.subTest(category=category):
                path = self.root / category / 'private.md'
                path.write_text(value)
                r = self.cli('scan', code=2)
                self.assertIn(category + '/private.md', r.stderr)
                self.assertNotIn(value, r.stderr)
                path.unlink()

    def test_staged_scanner_covers_all_visible_spec_categories(self):
        value = 'sk-' + 'd' * 30
        for category in sdd.SPEC_DIRS:
            with self.subTest(category=category):
                path = self.repo / category / 'private.md'
                path.parent.mkdir(exist_ok=True)
                path.write_text(value)
                relative = path.relative_to(self.repo).as_posix()
                self.g('add', relative)
                path.write_text('clean working copy')
                findings = staged_findings(self.repo)
                self.assertTrue(any(relative in line and 'api-token' in line for line in findings))
                self.assertNotIn(value, str(findings))
                self.g('reset', '--', relative)

    def test_visible_feature_symlink_cannot_escape_workspace(self):
        outside = Path(self.tmp.name) / 'outside-feature'
        outside.mkdir()
        (self.root / 'features/CHG-002').symlink_to(outside, target_is_directory=True)
        r = self.cli('status', 'CHG-002', code=2)
        self.assertIn('变更路径越界', r.stderr)

    def test_installer_idempotent_and_preserves_conflicts(self):
        src, dst = Path(self.tmp.name) / 'skills', Path(self.tmp.name) / 'installed'
        (src / 'demo').mkdir(parents=True)
        (src / 'demo/SKILL.md').write_text('original')
        install(src, dst)
        install(src, dst)
        (dst / 'demo/SKILL.md').write_text('user customization')
        with self.assertRaises(ValueError): install(src, dst)
        self.assertEqual((dst / 'demo/SKILL.md').read_text(), 'user customization')


class SameRepoWorkspace(Fixture, unittest.TestCase):
    """规格与代码同仓（本工程的实际布局）：账本自身写入不得使证据过期。"""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / 'mono'
        self.root.mkdir()
        self.repo = self.root
        self.g('init', '-b', 'main')
        (self.root / 'calc.py').write_text('def add(a, b):\n    return a + b\n')
        (self.root / 'test_calc.py').write_text('from calc import add\nassert add(1, 2) == 3\n')
        self.g('add', '.')
        self.g('-c', 'user.name=SDD Test', '-c', 'user.email=test@example.com', 'commit', '-qm', 'fixture')
        sdd.init(self.root)
        self.cli('repo', 'app', '--path', str(self.root), '--branch', 'main', '--role', '同仓样本')
        self.cli('new', 'CHG-001', '--title', '同仓证据保鲜', '--repos', 'app')
        self.p = self.root / 'features/CHG-001'
        (self.p / 'spec.md').write_text('# 测试规格\n\n### REQ-001: 两个整数相加\nWHEN 1 和 2 相加 THEN SHALL 返回 3。\n')

    def test_ledger_writes_keep_evidence_fresh(self):
        self.verify()
        self.cli('validate', 'CHG-001')
        events = sdd.read(self.p / 'evidence.json')['events']
        self.assertEqual([e['result'] for e in events], ['passed'])

    def test_state_refresh_does_not_invalidate_evidence(self):
        self.implementing()
        self.cli('task-done', 'CHG-001', 'T-001', '--code', 'calc.py', 'test_calc.py')
        self.cli('check', 'CHG-001', 'V-001')
        self.cli('refresh', 'CHG-001')
        self.cli('transition', 'CHG-001', 'VERIFIED')

    def test_source_change_still_invalidates_evidence(self):
        self.verify()
        (self.repo / 'calc.py').write_text('def add(a, b):\n    return a - b\n')
        self.cli('validate', 'CHG-001', code=2)

    def test_new_untracked_source_still_invalidates_evidence(self):
        self.verify()
        (self.repo / 'extra.py').write_text('print(42)\n')
        self.cli('validate', 'CHG-001', code=2)


if __name__ == '__main__':
    unittest.main()
