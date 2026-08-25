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

    def test_timeline_projection_removes_per_event_lists(self):
        result = {
            "fingerprint_hex": "0123456789abcdef",
            "fight_time": 300.0,
            "active_event_count": 2,
            "triggered_event_count": 1,
            "skills": [{"skill_id": 1}],
            "total_cd_wait_seconds": 1.5,
            "cd_waits": [{"cast_time": 1.0}],
            "total_observed_gcd_gap_seconds": 0.5,
            "gcd_gaps": [{"next_cast_time": 2.0}],
            "rage": {"minimum": 0, "maximum": 100},
            "buff_coverage": [
                {
                    "buff_id": 42,
                    "active_seconds": 10.0,
                    "coverage_percent": 50.0,
                    "activation_count": 1,
                    "intervals": [{"start": 0.0, "end": 10.0}],
                }
            ],
            "skipped": [],
            "limitations": ["not_causal"],
        }
        projection = benchmark_agent_tools.timeline_projection(result)

        self.assertEqual(projection["cd_wait_count"], 1)
        self.assertEqual(projection["gcd_gap_count"], 1)
        self.assertNotIn("cd_waits", projection)
        self.assertNotIn("intervals", projection["buff_coverage"][0])


if __name__ == "__main__":
    unittest.main()
