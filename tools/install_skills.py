#!/usr/bin/env python3
"""Install versioned skill folders; refuse to overwrite different existing content."""
import argparse
import hashlib
from pathlib import Path
import shutil


def manifest(path):
    return {str(p.relative_to(path)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in path.rglob('*') if p.is_file()}


def install(source, dest):
    pairs = [(p, dest / p.name) for p in sorted(source.iterdir()) if (p / 'SKILL.md').is_file()]
    for src, target in pairs:
        if target.exists() or target.is_symlink():
            if not target.is_dir() or manifest(src) != manifest(target):
                raise ValueError(f'已有不同版本，不覆盖：{target}。先备份并手动处理冲突。')
    dest.mkdir(parents=True, exist_ok=True)
    for src, target in pairs:
        if not target.exists():
            shutil.copytree(src, target)
        print(target)


if __name__ == '__main__':
    p = argparse.ArgumentParser(description='安装 SDD 技能；仅新增，不覆盖已有不同版本')
    p.add_argument('--dest', type=Path, default=Path.home() / '.codex/skills')
    a = p.parse_args()
    try:
        install(Path(__file__).resolve().parents[1] / 'skills', a.dest.expanduser())
    except (ValueError, OSError) as e:
        p.exit(2, str(e) + '\n')
