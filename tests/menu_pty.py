"""Exercise the actual CLI in a PTY using only Python's standard library."""
import errno
from datetime import datetime, timezone
import fcntl
import os
from pathlib import Path
import pty
import re
import select
import subprocess
import struct
import termios
import tempfile
import time
import unittest

BINARY = os.environ["SUBREPO_TEST_BINARY"]


class Terminal:
    def __init__(self, cwd, env, args=("pr",)):
        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 160, 0, 0))
        self.process = subprocess.Popen([BINARY, *args], cwd=cwd, env=env, stdin=slave,
                                        stdout=slave, stderr=slave)
        os.close(slave)
        self.output = ""
        self.position = 0

    def expect(self, needle, timeout=15):
        deadline = time.monotonic() + timeout
        while needle not in self.output[self.position:]:
            if time.monotonic() >= deadline:
                raise AssertionError(f"Missing {needle!r}: {self.output}")
            if select.select([self.master], [], [], 0.1)[0]:
                try:
                    chunk = os.read(self.master, 65536)
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    chunk = b""
                if not chunk:
                    raise AssertionError(f"Terminal exited before {needle!r}: {self.output}")
                self.output += chunk.decode(errors="replace")
        self.position = self.output.index(needle, self.position) + len(needle)

    def send(self, text):
        os.write(self.master, text.encode())

    def wait(self, timeout=10):
        # Selectors repaint while moving. Drain the PTY so a full output buffer
        # cannot block the child's final redraw and exit.
        deadline = time.monotonic() + timeout
        while self.process.poll() is None:
            if time.monotonic() >= deadline:
                raise AssertionError(f"Terminal did not exit: {self.output}")
            if select.select([self.master], [], [], 0.1)[0]:
                try:
                    self.output += os.read(self.master, 65536).decode(errors="replace")
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
        return self.process.returncode

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
        self.process.wait(timeout=10)
        os.close(self.master)


class GuidedMenu(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="subrepo-menu-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.owner = self.root / "owner"
        self.upstream = self.root / "upstream"
        self.env = dict(os.environ, GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_SYSTEM="/dev/null",
                        GIT_AUTHOR_NAME="Test", GIT_AUTHOR_EMAIL="test@example.com",
                        GIT_COMMITTER_NAME="Test", GIT_COMMITTER_EMAIL="test@example.com",
                        GIT_EDITOR="true", SHELL="/bin/sh", TERM="xterm", NO_COLOR="1")
        self.env.pop("GIT_SUBREPO_WORKON", None)
        for directory in [self.owner, self.upstream]:
            directory.mkdir()
            self.git(directory, "init", "-q", "-b", "main")
            (directory / "file").write_text("original\n")
            self.git(directory, "add", "file")
            self.git(directory, "commit", "-qm", "Initial")
        self.cli(self.owner, "clone", str(self.upstream), "lib/unit", "--no-edit")
        (self.owner / "lib/unit/file").write_text("local change\n")
        self.git(self.owner, "add", "lib/unit/file")
        self.git(self.owner, "commit", "-qm", "Local change")

    def command(self, directory, *args):
        return subprocess.check_output(args, cwd=directory, env=self.env, stderr=subprocess.STDOUT).decode().strip()

    def git(self, directory, *args):
        return self.command(directory, "git", *args)

    def cli(self, directory, *args):
        return self.command(directory, BINARY, *args)

    def terminal(self, directory, args=("pr",)):
        terminal = Terminal(directory, self.env, args)
        self.addCleanup(terminal.close)
        return terminal

    def prepare(self):
        return Path(self.cli(self.owner, "pr", "prepare", "lib/unit", "--branch",
                             "contribute/existing", "--message", "Public change", "--quiet"))

    def choose(self, terminal, label):
        start = terminal.position
        terminal.expect("Quit / back")
        screen = re.sub(r"\x1b\[[0-9;?]*[A-Za-z]", "", terminal.output[start:terminal.position])
        labels = [line.strip().lstrip("❯> ") for line in screen.splitlines()]
        options = [line for line in labels if line in {
            "Open existing export", "Review export differences", "Review local changes",
            "Prepare new contribution", "Publish reviewed export",
            "Bring accepted upstream changes back", "Quit / back"}]
        target = "Quit / back" if label == "Quit" else label
        self.assertIn(target, options, screen)
        terminal.send("\x1b[B" * options.index(target) + "\r")

    def test_resume_from_dirty_sibling_does_not_rebuild_export(self):
        export = self.prepare()
        (export / "file").write_text("unfinished review fix\n")
        sibling = self.root / "dirty checkout"
        self.git(self.owner, "worktree", "add", "-b", "later", str(sibling))
        (sibling / "lib/unit/file").write_text("unfinished source\n")
        before = self.git(sibling, "status", "--porcelain")
        terminal = self.terminal(sibling)
        self.choose(terminal, "Open existing export")
        terminal.expect("Type exit to return")
        terminal.send("pwd\n")
        terminal.expect(str(export))
        terminal.send("exit\n")
        self.choose(terminal, "Quit")
        self.assertEqual(terminal.wait(), 0)
        self.assertEqual((export / "file").read_text(), "unfinished review fix\n")
        self.assertEqual(self.git(sibling, "status", "--porcelain"), before)
        self.assertIn(str(self.owner), terminal.output)

    def test_prepare_prompts_then_publication_defaults_to_no(self):
        before = self.git(self.owner, "rev-parse", "HEAD")
        terminal = self.terminal(self.owner)
        self.choose(terminal, "Prepare new contribution")
        terminal.expect("Topic branch")
        terminal.send("contribute/menu-test\n")
        terminal.expect("Public commit message")
        terminal.send("Public menu contribution\n")
        terminal.expect("Upstream URL")
        terminal.send("\n")
        terminal.expect("Base branch")
        terminal.send("\n")
        terminal.expect("Push URL")
        terminal.send("\n")
        terminal.expect("Prepare this contribution?")
        terminal.send("y")
        self.choose(terminal, "Publish reviewed export")
        terminal.expect("Publish this exact revision?")
        terminal.send("\n")
        self.choose(terminal, "Quit")
        self.assertEqual(terminal.wait(), 0)
        self.assertEqual(self.git(self.owner, "rev-parse", "HEAD"), before)
        status = self.cli(self.owner, "pr", "status", "lib/unit")
        self.assertIn("branch: contribute/menu-test", status)
        self.assertNotIn("published:", status)
        self.assertNotIn("contribute/menu-test", self.git(self.upstream, "branch", "--list"))

    def test_dirty_prepare_shows_files_without_committing(self):
        path = self.owner / "lib/unit/file"
        path.write_text("unfinished\n")
        before = self.git(self.owner, "rev-parse", "HEAD")
        terminal = self.terminal(self.owner)
        self.choose(terminal, "Prepare new contribution")
        terminal.expect("lib/unit/file")
        self.choose(terminal, "Quit")
        self.assertEqual(terminal.wait(), 0)
        self.assertEqual(self.git(self.owner, "rev-parse", "HEAD"), before)
        self.assertEqual(path.read_text(), "unfinished\n")

    def test_publication_refuses_a_revision_changed_during_confirmation(self):
        export = self.prepare()
        terminal = self.terminal(self.owner)
        self.choose(terminal, "Publish reviewed export")
        terminal.expect("Publish this exact revision?")
        (export / "file").write_text("new review fix\n")
        self.git(export, "add", "file")
        self.git(export, "commit", "-qm", "Follow-up")
        terminal.send("y")
        terminal.expect("Candidate changed since review")
        self.choose(terminal, "Quit")
        self.assertEqual(terminal.wait(), 0)
        self.assertNotIn("contribute/existing", self.git(self.upstream, "branch", "--list"))

    def test_export_menu_publishes_only_after_explicit_confirmation(self):
        export = self.prepare()
        owner_head = self.git(self.owner, "rev-parse", "HEAD")
        metadata = (self.owner / "lib/unit/.gitrepo").read_bytes()
        terminal = self.terminal(export)
        self.choose(terminal, "Publish reviewed export")
        terminal.expect("Publish this exact revision?")
        terminal.send("y")
        terminal.expect("Published reviewed commit")
        self.choose(terminal, "Quit")
        self.assertEqual(terminal.wait(), 0)
        self.assertEqual(self.git(self.upstream, "rev-parse", "contribute/existing"),
                         self.git(export, "rev-parse", "HEAD"))
        self.assertEqual(self.git(self.owner, "rev-parse", "HEAD"), owner_head)
        self.assertEqual((self.owner / "lib/unit/.gitrepo").read_bytes(), metadata)

    def test_escape_exits_without_creating_an_export(self):
        terminal = self.terminal(self.owner)
        terminal.expect("Quit / back")
        terminal.send("\x1b")
        self.assertEqual(terminal.wait(), 0)
        self.assertFalse((self.owner / ".git/subrepo-pr").exists())

    def test_confirmation_cannot_publish_a_replacement_session(self):
        original = self.prepare()
        head = self.git(original, "rev-parse", "HEAD")
        terminal = self.terminal(self.owner)
        self.choose(terminal, "Publish reviewed export")
        terminal.expect("Publish this exact revision?")
        self.cli(self.owner, "pr", "close", "lib/unit")
        replacement = Path(self.cli(self.owner, "pr", "prepare", "lib/unit", "--branch",
                                    "contribute/replacement", "--message", "Replacement", "--quiet"))
        self.git(replacement, "fetch", str(original), head)
        self.git(replacement, "reset", "--hard", head)
        terminal.send("y")
        self.choose(terminal, "Quit")
        self.assertEqual(terminal.wait(), 0)
        self.assertNotIn("contribute/", self.git(self.upstream, "branch", "--list"))
        self.assertIn("Export session changed", terminal.output)

    def test_non_terminal_still_prints_help(self):
        output = self.cli(self.owner)
        self.assertIn("Usage:", output)
        self.assertNotIn("Choice", output)

    def test_bare_command_prints_help_even_in_terminal(self):
        terminal = self.terminal(self.owner, args=())
        terminal.expect("Usage:")
        self.assertEqual(terminal.wait(), 0)

    def test_non_terminal_pr_prints_instructions(self):
        output = self.cli(self.owner, "pr")
        self.assertIn("git subrepo pr --help", output)
        self.assertNotIn("Select action", output)

    def test_nested_config_write_defaults_to_no_without_changing_metadata(self):
        cwd = self.owner / "lib/unit"
        before = (cwd / ".gitrepo").read_bytes()
        terminal = self.terminal(cwd, args=("config", ".", "exclude", "LOCAL_PATCHES.md"))
        terminal.expect("Checkout:")
        terminal.expect("Target: lib/unit")
        terminal.expect("Execute in this checkout?")
        terminal.send("\n")
        self.assertEqual(terminal.wait(), 0)
        self.assertEqual((cwd / ".gitrepo").read_bytes(), before)

    def test_nested_config_accepts_confirmed_root_relative_fallback(self):
        cwd = self.owner / "lib/unit"
        terminal = self.terminal(cwd, args=("config", "lib/unit", "exclude", "LOCAL_PATCHES.md"))
        terminal.expect("checkout-root-relative")
        terminal.expect("Execute in this checkout?")
        terminal.send("y")
        self.assertEqual(terminal.wait(), 0)
        self.assertEqual(self.git(self.owner, "config", "--file", "lib/unit/.gitrepo", "subrepo.exclude"), "LOCAL_PATCHES.md")

    def test_nested_config_read_does_not_prompt(self):
        terminal = self.terminal(self.owner / "lib/unit", args=("config", ".", "method"))
        terminal.expect("option 'method'")
        self.assertEqual(terminal.wait(), 0)
        self.assertNotIn("Execute in this checkout?", terminal.output)

    def test_yes_only_skips_nested_execution_prompt(self):
        terminal = self.terminal(self.owner / "lib/unit", args=("config", ".", "exclude", "local", "--yes"))
        terminal.expect("option 'exclude'")
        self.assertEqual(terminal.wait(), 0)
        self.assertNotIn("Execute in this checkout?", terminal.output)

    def test_quiet_nested_confirmation_still_displays_target(self):
        terminal = self.terminal(self.owner / "lib/unit", args=("-q", "config", ".", "exclude", "local"))
        terminal.expect("Checkout:")
        terminal.expect("Target: lib/unit")
        terminal.expect("Execute in this checkout?")
        terminal.send("\n")
        self.assertEqual(terminal.wait(), 0)

    def test_export_menu_displays_creation_source_and_recorded_policy(self):
        source = self.git(self.owner, "rev-parse", "HEAD")
        export = self.prepare()
        session = export.parents[2] / "session"
        self.git(self.owner, "config", "--file", str(session), "export.created-at", "2026-10-14 08:45:00 UTC")
        before = session.read_bytes()
        terminal = self.terminal(self.owner)
        terminal.expect("Workflow:")
        terminal.expect("Created: 2026-10-14 08:45:00 UTC")
        terminal.expect("Source: " + source[:12])
        terminal.expect("Recorded exclusions: none")
        self.choose(terminal, "Quit")
        self.assertEqual(terminal.wait(), 0)
        self.assertEqual(session.read_bytes(), before)

    def test_legacy_export_date_and_policy_mismatch_are_visible_from_sibling(self):
        export = self.prepare()
        session = export.parents[2] / "session"
        # Simulate an export prepared before date/policy fields existed.
        for key in ("created-at", "source-branch", "exclude-count"):
            subprocess.run(["git", "config", "--file", str(session), "--unset-all", "export." + key],
                           cwd=self.owner, env=self.env, check=False, capture_output=True)
        sibling = self.root / "different checkout"
        self.git(self.owner, "worktree", "add", "-b", "later", str(sibling))
        self.git(sibling, "config", "--file", "lib/unit/.gitrepo", "subrepo.exclude", "LOCAL_PATCHES.md")
        before = session.read_bytes()
        expected = datetime.fromtimestamp(int(export.parent.name) // 1_000_000_000,
                                          tz=timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC")
        terminal = self.terminal(sibling)
        terminal.expect('Current exclusions: "LOCAL_PATCHES.md"')
        terminal.expect("Created: " + expected)
        terminal.expect("legacy session ID")
        terminal.expect("Recorded exclusions: not recorded")
        terminal.expect("Exclusions differ from this checkout")
        self.choose(terminal, "Quit")
        self.assertEqual(terminal.wait(), 0)
        self.assertEqual(session.read_bytes(), before)
        self.assertEqual((export / "file").read_text(), "local change\n")


if __name__ == "__main__":
    unittest.main()
