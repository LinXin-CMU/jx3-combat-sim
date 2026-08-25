import json
import unittest
from collections import Counter
from pathlib import Path

import run_agent_eval


TESTS_ROOT = Path(__file__).resolve().parent


class AgentEvalFixtureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.cases = json.loads(
            (TESTS_ROOT / "agent_eval" / "cases.json").read_text(encoding="utf-8")
        )["cases"]
        cls.scenarios = json.loads(
            (TESTS_ROOT / "agent_eval" / "scenarios.json").read_text(encoding="utf-8")
        )["scenarios"]

    def test_fixture_schema_and_numeric_provenance(self):
        run_agent_eval.validate_fixture_schema(self.cases, self.scenarios)

    def test_exact_category_contract(self):
        counts = Counter(case["category"] for case in self.cases)
        self.assertEqual(counts, Counter(run_agent_eval.EXPECTED_CATEGORIES))

    def test_all_exposed_tools_are_read_only(self):
        exposed = {
            tool
            for case in self.cases
            for tool in case["allowed_tools"]
        }
        self.assertLessEqual(exposed, run_agent_eval.READ_ONLY_TOOLS)

    def test_assertion_engine_checks_cross_path_and_delta(self):
        root = {"baseline": {"dps": 100.0}, "candidate": {"dps": 125.0, "delta": 25.0}}
        run_agent_eval.assert_rule(
            root,
            {
                "path": "candidate.delta",
                "op": "approx_delta",
                "minuend_path": "candidate.dps",
                "subtrahend_path": "baseline.dps",
            },
        )
        run_agent_eval.assert_rule(
            root,
            {"path": "candidate.delta", "op": "eq_path", "value": "candidate.delta"},
        )


if __name__ == "__main__":
    unittest.main()
