"""Exercise tag provenance with real Git refs, including checkout's peeled tag."""
from contextlib import ExitStack
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import build_retrom_candidate as candidate
import build_retrom_release as release


class BuildReached(Exception):
    pass


class ReleaseTagTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="siglus-tag-test-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.git("init", "-q")
        self.git("config", "user.name", "Release fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        self.git("commit", "--allow-empty", "-qm", "fixture")
        self.tag = "retrom-core-g5f358ef1360b-r2"
        self.raw = "refs/retrom-release/" + self.tag
        self.git("branch", "retrom/g5f358ef1360b")
        self.git("tag", "-a", self.tag, "-m", "annotated fixture")
        self.git("update-ref", self.raw, self.git("rev-parse", self.tag))
        # actions/checkout fetches +<commit>:refs/tags/<name> after its full fetch.
        self.git("update-ref", "refs/tags/" + self.tag, "HEAD")
        stack = ExitStack()
        self.addCleanup(stack.close)
        stack.enter_context(patch.object(candidate, "ROOT", self.root))
        stack.enter_context(patch.object(release, "ROOT", self.root))
        stack.enter_context(patch.object(release, "BASELINE", self.git("rev-parse", "HEAD")))
        stack.enter_context(patch.object(release, "build_candidate", side_effect=BuildReached))
        stack.enter_context(patch.object(sys, "argv", ["build_retrom_release.py", str(self.root / "output"), self.tag]))
        stack.enter_context(patch.dict("os.environ", {"RETROM_RELEASE_TAG_REF": self.raw}))

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.root, stderr=subprocess.PIPE).decode().strip()

    def test_original_annotated_tag_survives_checkout_peeling(self):
        self.assertEqual(self.git("cat-file", "-t", self.tag), "commit")
        self.assertEqual(self.git("cat-file", "-t", self.raw), "tag")
        with self.assertRaises(BuildReached):
            release.main()

    def test_lightweight_remote_tag_is_rejected(self):
        self.git("update-ref", self.raw, "HEAD")
        with self.assertRaisesRegex(ValueError, "SIGLUS_ANNOTATED_TAG_REQUIRED"):
            release.main()

    def test_annotated_tag_at_another_commit_is_rejected(self):
        self.git("commit", "--allow-empty", "-qm", "different commit")
        with self.assertRaisesRegex(ValueError, "SIGLUS_ANNOTATED_TAG_REQUIRED"):
            release.main()


if __name__ == "__main__":
    unittest.main()
