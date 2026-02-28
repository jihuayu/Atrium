#!/usr/bin/env python3
"""
scripts/deploy.py — xtalk Worker 部署脚本

自动完成：构建 → D1 迁移 → 部署

Usage:
    python scripts/deploy.py              # 构建 + 迁移 + 部署
    python scripts/deploy.py --dry-run    # 只构建 + 显示待执行迁移，不实际部署
    python scripts/deploy.py --migrate    # 只执行迁移（不构建、不部署）
    python scripts/deploy.py --build      # 只构建（不迁移、不部署）

前提条件:
    - npx wrangler login（或设置 CLOUDFLARE_API_TOKEN 环境变量）
    - cargo, worker-build 已安装
"""

import argparse
import re
import subprocess
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).parent.resolve()
PROJECT_ROOT = SCRIPT_DIR.parent
WORKER_DIR = PROJECT_ROOT / "deploy" / "worker"

# Windows 上 npx/npm 是 .cmd 批处理
SHELL = sys.platform == "win32"
NPX = ["npx", "-y"]


def safe_print(text: str) -> None:
    """Print text safely on Windows (GBK console can't render emoji/unicode)."""
    try:
        print(text)
    except UnicodeEncodeError:
        print(text.encode("ascii", errors="replace").decode("ascii"))


def banner(msg: str) -> None:
    print(f"\n{'=' * 50}\n  {msg}\n{'=' * 50}")


def run(cmd: list[str], **kwargs) -> subprocess.CompletedProcess:
    print(f"  $ {' '.join(str(c) for c in cmd)}")
    return subprocess.run(cmd, check=True, shell=SHELL, **kwargs)


def step_build() -> None:
    banner("Step 1: Build Worker WASM")
    run(["cargo", "install", "-q", "worker-build"], cwd=PROJECT_ROOT)
    run(["worker-build", "--release", "--features", "worker"], cwd=PROJECT_ROOT)
    print("  Build complete.")


def step_migrate(dry_run: bool = False) -> None:
    banner("Step 2: Apply D1 migrations")

    # 先查看待执行的迁移
    print("  Checking pending migrations...")
    result = subprocess.run(
        NPX + ["wrangler", "d1", "migrations", "list", "DB",
               "--config", str(WORKER_DIR / "wrangler.toml"), "--remote"],
        cwd=WORKER_DIR,
        shell=SHELL,
        capture_output=True,
        encoding="utf-8",
        errors="replace",
    )
    if result.stdout.strip():
        safe_print(result.stdout)
    if result.stderr:
        for line in result.stderr.splitlines():
            if "error" in line.lower():
                print(f"  {line}")

    if dry_run:
        print("  [dry-run] Skipping actual migration apply.")
        return

    # 执行迁移
    print("  Applying migrations to remote D1...")
    run(
        NPX + ["wrangler", "d1", "migrations", "apply", "DB",
               "--config", str(WORKER_DIR / "wrangler.toml"), "--remote"],
        cwd=WORKER_DIR,
    )
    print("  Migrations applied.")


def step_deploy(dry_run: bool = False) -> None:
    banner("Step 3: Deploy Worker")

    if dry_run:
        print("  [dry-run] Would run: npx wrangler deploy")
        return

    run(
        NPX + ["wrangler", "deploy", "--config", str(WORKER_DIR / "wrangler.toml")],
        cwd=WORKER_DIR,
    )
    print("  Worker deployed.")


def main() -> None:
    parser = argparse.ArgumentParser(description="xtalk Worker deploy script")
    parser.add_argument("--dry-run", action="store_true",
                        help="Build + show pending migrations, but don't deploy")
    parser.add_argument("--migrate", action="store_true",
                        help="Only run D1 migrations (skip build & deploy)")
    parser.add_argument("--build", action="store_true",
                        help="Only build WASM (skip migrate & deploy)")
    args = parser.parse_args()

    if args.build:
        step_build()
        return

    if args.migrate:
        step_migrate(dry_run=args.dry_run)
        return

    # 默认：全流程
    step_build()
    step_migrate(dry_run=args.dry_run)
    step_deploy(dry_run=args.dry_run)
    print(f"\n  Done.")


if __name__ == "__main__":
    main()
