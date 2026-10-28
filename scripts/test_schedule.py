"""Tests of the schedule's table generator (`schedule.py`, FD-045-D13):

    python3 -I -m unittest discover -s scripts -p 'test_schedule.py'
"""

import io
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

import schedule as sc

SCHEDULE = '''\
//! The schedule.

/// A world step.
pub enum WorldStep {
    /// The commands run (`BaseWorld::update`).
    Commands,
    /// The turn passes, see [`World::turn`].
    Turn,
}

/// A player's step.
pub enum PlayerStep {
    /// The player's objects update in their list's order (`FUN_141166550`).
    Objects,
}

/// Before.
pub const BEFORE_PLAYERS: [WorldStep; 1] = [WorldStep::Commands];

/// Each player's.
pub const EACH_PLAYER: [PlayerStep; 1] = [PlayerStep::Objects];

/// After.
pub const AFTER_PLAYERS: [WorldStep; 1] = [
    WorldStep::Turn,
];
'''

DOC = "# Systems\n\n<!-- schedule -->\nold\n<!-- /schedule -->\n\nAfter.\n"


def run(src: Path, *args: str) -> tuple[int, str]:
    err = io.StringIO()
    with redirect_stdout(io.StringIO()), redirect_stderr(err):
        code = sc.main([str(src), *args])
    return code, err.getvalue()


class ScheduleTests(unittest.TestCase):
    def test_the_variants_come_with_their_docs_in_order(self):
        self.assertEqual(
            sc.variants(SCHEDULE, "WorldStep"),
            {"Commands": "The commands run (`BaseWorld::update`).", "Turn": "The turn passes, see [`World::turn`]."},
        )

    def test_the_arrays_list_the_steps_in_the_file_s_order(self):
        self.assertEqual(
            sc.arrays(SCHEDULE),
            [
                ("BEFORE_PLAYERS", "WorldStep", ["Commands"]),
                ("EACH_PLAYER", "PlayerStep", ["Objects"]),
                ("AFTER_PLAYERS", "WorldStep", ["Turn"]),
            ],
        )

    def test_the_table_numbers_the_steps_and_says_how_often_they_run(self):
        self.assertEqual(
            sc.table(SCHEDULE).split("\n"),
            [
                "| # | Step | Runs | What it does |",
                "| ---: | --- | --- | --- |",
                "| 1 | `Commands` | once | The commands run (`BaseWorld::update`). |",
                "| 2 | `Objects` | for each player | The player's objects update in their list's order (`FUN_141166550`). |",
                "| 3 | `Turn` | once | The turn passes, see `World::turn`. |",
            ],
        )

    def test_a_variant_left_out_or_held_twice_is_a_problem(self):
        left_out = SCHEDULE.replace("[WorldStep::Commands]", "[WorldStep::Turn]")
        self.assertEqual(
            sc.problems_of(left_out),
            ["WorldStep::Commands is not scheduled", "WorldStep::Turn is scheduled twice"],
        )
        self.assertEqual(sc.problems_of(SCHEDULE.replace("PlayerStep::Objects]", "PlayerStep::Fly]")),
                         ["PlayerStep::Objects is not scheduled", "PlayerStep::Fly is no variant"])
        self.assertEqual(sc.problems_of(SCHEDULE), [])

    def test_check_fails_until_the_section_is_written(self):
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / "src"
            (src / "world").mkdir(parents=True)
            (src / "world" / "schedule.rs").write_text(SCHEDULE)
            out = Path(d) / "SYSTEMS.md"
            out.write_text(DOC)
            code, err = run(src, "--out", str(out), "--check")
            self.assertEqual(code, 1)
            self.assertIn("run `just schedule`", err)
            self.assertEqual(run(src, "--out", str(out)), (0, ""))
            self.assertIn("| 3 | `Turn` |", out.read_text())
            self.assertTrue(out.read_text().endswith("<!-- /schedule -->\n\nAfter.\n"))
            self.assertEqual(run(src, "--out", str(out), "--check"), (0, ""))


if __name__ == "__main__":
    unittest.main()
