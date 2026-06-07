import pathlib
import unittest


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]
RUN = REPO_ROOT / "ralph-scripts" / "run.sh"
README = REPO_ROOT / "ralph-scripts" / "README.md"


class RunOrchestratorTest(unittest.TestCase):
    def test_run_sh_orchestrates_test_triage_build_outer_cycles(self):
        script = RUN.read_text()

        self.assertIn("MAX_OUTER_CYCLES", script)
        self.assertIn("run_tests_and_file_beads", script)
        self.assertIn("consolidate_and_investigate_beads", script)
        self.assertIn("run_build_until_no_open_beads", script)
        self.assertIn("pptx_roundtrip_e2e.py", script)
        self.assertIn("PROMPT_consolidate.md", script)
        self.assertIn("loop.sh", script)
        self.assertIn("bd list --status=open", script)
        self.assertIn("No new beads were created", script)

    def test_readme_documents_outer_runner(self):
        readme = README.read_text()

        self.assertIn("./run.sh", readme)
        self.assertIn("outer", readme.lower())
        self.assertIn("consolidate", readme.lower())
        self.assertIn("roundtrip", readme.lower())


if __name__ == "__main__":
    unittest.main()
