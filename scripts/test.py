#!/usr/bin/env python3
"""
scripts/test.py — Atrium test runner

Usage:
    python scripts/test.py server
    python scripts/test.py worker
    python scripts/test.py all
    python scripts/test.py worker compat_issues
"""

import argparse
import os
import re
import secrets
import signal
import shutil
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

SCRIPT_DIR = Path(__file__).parent.resolve()
PROJECT_ROOT = SCRIPT_DIR.parent
WORKER_DIR = PROJECT_ROOT / "deploy" / "worker"
MIGRATIONS_DIR = PROJECT_ROOT / "migrations"

# Windows 上 npx/npm 等是 .cmd 批处理，需要 shell=True 才能被 subprocess 找到
SHELL = sys.platform == "win32"

# npx -y 跳过 "Need to install..." 安装确认提示
NPX = ["npx", "-y"]


def kill_port_holders(port: int) -> None:
    """Kill any processes listening on the given port (stale orphans from previous runs)."""
    if sys.platform != "win32":
        return
    try:
        out = subprocess.check_output(
            ["netstat", "-ano", "-p", "TCP"],
            text=True, stderr=subprocess.DEVNULL,
        )
    except Exception:
        return
    pids: set[int] = set()
    for line in out.splitlines():
        if f"127.0.0.1:{port}" in line and "LISTENING" in line:
            parts = line.split()
            try:
                pids.add(int(parts[-1]))
            except (ValueError, IndexError):
                pass
    for pid in pids:
        print(f"  Killing stale process on :{port} (PID {pid})")
        subprocess.run(
            ["taskkill", "/T", "/F", "/PID", str(pid)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )


def banner(msg: str) -> None:
    print(f"\n{'=' * 50}\n  {msg}\n{'=' * 50}")


def run(cmd: list[str], **kwargs) -> None:
    print("  $ " + " ".join(str(c) for c in cmd))
    subprocess.run(cmd, check=True, shell=SHELL, **kwargs)


def run_server_tests(extra: list[str]) -> None:
    banner("Running SERVER tests")
    run(
        [
            "cargo",
            "test",
            "--features",
            "server,test-utils",
            "--test",
            "integration_test",
        ]
        + extra,
        cwd=PROJECT_ROOT,
    )


def run_worker_tests(extra: list[str]) -> None:
    banner("Running WORKER tests")

    test_port = 8788
    # 清除之前运行残留的僵尸进程
    kill_port_holders(test_port)

    bypass_secret = secrets.token_hex(16)
    jwt_secret = secrets.token_urlsafe(32)
    # 本地模式用固定 dummy ID，wrangler dev --local 自动创建本地 SQLite D1
    dummy_db_id = "00000000-0000-0000-0000-000000000000"
    dummy_db_name = "atrium-test-local"

    wrangler_proc: subprocess.Popen | None = None
    temp_cfg_path: str | None = None
    local_d1_dir: Path | None = None

    def kill_proc_tree(pid: int, *, force: bool = False) -> None:
        """Kill a process and all its children. On Windows with shell=True,
        terminate() only kills cmd.exe — child node/workerd processes survive."""
        if sys.platform == "win32":
            subprocess.run(
                ["taskkill", "/T", "/F", "/PID", str(pid)],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        else:
            try:
                sig = signal.SIGKILL if force else signal.SIGTERM
                os.killpg(os.getpgid(pid), sig)
            except (ProcessLookupError, PermissionError):
                pass

    def cleanup() -> None:
        print("\n--- Cleanup ---")
        if wrangler_proc and wrangler_proc.poll() is None:
            print(f"Stopping wrangler dev (PID {wrangler_proc.pid})...")
            kill_proc_tree(wrangler_proc.pid)
            try:
                wrangler_proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                kill_proc_tree(wrangler_proc.pid, force=True)

        if temp_cfg_path:
            Path(temp_cfg_path).unlink(missing_ok=True)

        # 清理本地 D1 数据（.wrangler/state）
        if local_d1_dir and local_d1_dir.exists():
            print(f"Cleaning up local D1 state: {local_d1_dir}")
            shutil.rmtree(local_d1_dir, ignore_errors=True)

    try:
        # ── Step 1: Build worker WASM ────────────────────────────
        print("\n[1/4] Building worker...")
        run(
            ["cargo", "install", "-q", "worker-build"],
            cwd=PROJECT_ROOT,
        )
        run(
            ["worker-build", "--release", "--features", "worker"],
            cwd=PROJECT_ROOT,
        )

        # ── Step 2: Generate temp wrangler config ────────────────
        print("\n[2/4] Generating test wrangler config...")
        template = (WORKER_DIR / "wrangler.test.toml.template").read_text(
            encoding="utf-8"
        )
        config = (
            template.replace("__TEST_DB_NAME__", dummy_db_name)
            .replace("__TEST_DB_ID__", dummy_db_id)
            .replace("__TEST_PORT__", str(test_port))
            .replace("__TEST_BYPASS_SECRET__", bypass_secret)
            .replace("__TEST_JWT_SECRET__", jwt_secret)
        )
        temp_cfg_path = str(WORKER_DIR / f".wrangler.test.{os.getpid()}.toml")
        with open(temp_cfg_path, "w", encoding="utf-8") as f:
            f.write(config)

        # 应用合并 schema 到本地 D1（单文件，避免迁移 rename/drop 与 D1 FK 冲突）
        test_init_sql = WORKER_DIR / "test_init.sql"
        print(f"  Applying schema from {test_init_sql.name}...")
        run(
            NPX
            + [
                "wrangler",
                "d1",
                "execute",
                "DB",
                "--config",
                temp_cfg_path,
                "--file",
                str(test_init_sql),
                "--local",
                "--yes",
            ],
            cwd=WORKER_DIR,
        )

        # 记录本地 D1 state 路径，用于清理
        local_d1_dir = WORKER_DIR / ".wrangler"

        # ── Step 3: Start wrangler dev (local mode) ──────────────
        print(f"\n[3/4] Starting wrangler dev on :{test_port} (local mode)...")
        # On POSIX, isolate wrangler in its own session so killpg only targets wrangler.
        wrangler_proc = subprocess.Popen(
            NPX
            + [
                "wrangler",
                "dev",
                "--config",
                temp_cfg_path,
                "--port",
                str(test_port),
            ],
            cwd=WORKER_DIR,
            shell=SHELL,
            start_new_session=(sys.platform != "win32"),
        )

        # 等待就绪（最多 60s）
        # 用 127.0.0.1 而非 localhost，Windows 上 localhost 可能解析到 IPv6 ::1
        print("  Waiting for wrangler dev to be ready...")
        for i in range(60):
            if wrangler_proc.poll() is not None:
                print(
                    f"ERROR: wrangler dev exited with code {wrangler_proc.returncode}"
                )
                sys.exit(1)
            try:
                urllib.request.urlopen(
                    f"http://127.0.0.1:{test_port}/", timeout=2
                )
                break
            except Exception:
                time.sleep(1)
        else:
            print("ERROR: wrangler dev did not start within 60 s")
            sys.exit(1)
        print("  wrangler dev ready")

        # ── Step 4: Run integration tests ────────────────────────
        print("\n[4/4] Running integration tests...")
        env = os.environ.copy()
        env["ATRIUM_TEST_BASE_URL"] = f"http://127.0.0.1:{test_port}"
        env["ATRIUM_TEST_BYPASS_SECRET"] = bypass_secret
        env["XTALK_TEST_BASE_URL"] = env["ATRIUM_TEST_BASE_URL"]
        env["XTALK_TEST_BYPASS_SECRET"] = env["ATRIUM_TEST_BYPASS_SECRET"]
        run(
            ["cargo", "test", "--test", "integration_test"] + extra,
            cwd=PROJECT_ROOT,
            env=env,
        )
    finally:
        cleanup()


def main() -> None:
    parser = argparse.ArgumentParser(description="Atrium test runner")
    parser.add_argument(
        "mode", nargs="?", default="all", choices=["server", "worker", "all"]
    )
    parser.add_argument("extra", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    if args.mode == "server":
        run_server_tests(args.extra)
    elif args.mode == "worker":
        run_worker_tests(args.extra)
    else:
        run_server_tests([])
        run_worker_tests([])
        print("\n All tests passed (server + worker)")


if __name__ == "__main__":
    main()
