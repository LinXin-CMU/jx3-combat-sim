import unittest

import benchmark_agent_tools


class AgentBenchmarkTests(unittest.TestCase):
    def test_percentile_uses_linear_interpolation(self):
        values = [1.0, 2.0, 3.0, 4.0]
        self.assertEqual(benchmark_agent_tools.percentile(values, 0.5), 2.5)
        self.assertAlmostEqual(benchmark_agent_tools.percentile(values, 0.95), 3.85)

    def test_distribution_reports_expected_fields(self):
        result = benchmark_agent_tools.distribution([1.0, 2.0, 3.0])
        self.assertEqual(set(result), {"p50", "p95", "mean", "min", "max"})
        self.assertEqual(result["p50"], 2.0)

    def test_stable_signature_ignores_duration(self):
        response = {
            "evidence": {
                "evidence_id": "evidence",
                "scenario_hash": "scenario",
                "duration_ms": 99,
                "result": {"fingerprint_hex": "0123456789abcdef"},
            }
        }
        self.assertEqual(
            benchmark_agent_tools.stable_signature("simulate_scenario", response),
            ("evidence", "scenario", "0123456789abcdef"),
        )


if __name__ == "__main__":
    unittest.main()
