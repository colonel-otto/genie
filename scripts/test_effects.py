"""Tests of the effects inventory's generator (`effects.py`, FD-045):

    python3 -I -m unittest discover -s scripts -p 'test_effects.py'
"""

import io
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

import effects as ef

CONTEXT = '''\
//! The step.

pub(crate) struct Step<'a> {
    /// The tick.
    pub(crate) tick: u32,
    /// The targets the update let go (slot 0x180, `FUN_140a7a230`), which the caller releases
    /// after the update.
    pub(crate) releases: Vec<(i32, Releaser)>,
    /// The completions: the caller makes the stack units.
    pub(crate) completions: Vec<(i32, Option<StackUnit>, bool)>,
}

pub(crate) struct ObjectOutputs {
    /// [`Step`]'s `releases`.
    pub(in crate::world) released: Vec<(i32, Releaser)>,
    /// [`Step`]'s `completions`.
    pub(in crate::world) completions:
        Vec<(i32, Option<StackUnit>, bool)>,
}
'''

APPLY = '''\
//! The world after a step.

impl World {
    fn apply_releases(&mut self, out: &mut ObjectOutputs) {
        for r in out.released.drain(..) {}
    }

    fn apply_completions(&mut self, out: &mut ObjectOutputs) {
        let _ = &out.completions;
    }
}
'''

UNIT_AI = '''\
//! The unit AI.

/// What the unit AI does to its unit.
pub enum Effect {
    /// stopAction (the unit's slot 0x3c0, `FUN_14108c160`): its actions go.
    Stop,
    /// The AI's move (slot 0x130), which the unit takes.
    Move(AiMove),
    /// The AI's move to an object.
    MoveToObject {
        /// The target.
        target: i32,
    },
}
'''

BUILD = '''\
//! The Build action.

use crate::unit_ai::Effect;

pub struct BuildStep {
    /// The units on the footprint, which its test (`FUN_14123bb20`) would send off: the
    /// caller's.
    pub occupants: Vec<i32>,
    /// The place, for the caller's use.
    pub place: [f32; 3],
    /// Its state.
    pub state: u8,
}

fn build(c: &mut Step<'_>, step: &BuildStep) {
    c.releases.push((1, by));
    let _ = Effect::Stop;
}
'''

READER = '''\
//! The builder.

fn after(step: BuildStep) {
    let _ = step.occupants;
    let _ = Effect::Move(m);
}
'''


def crate(d: Path) -> Path:
    src = d / "src"
    for path, text in {
        "world/step.rs": CONTEXT,
        "world/after_step.rs": APPLY,
        "unit_ai/mod.rs": UNIT_AI,
        "construction/build.rs": BUILD,
        "construction/unit.rs": READER,
        "construction/tests.rs": "use super::*;\nfn t(c: &mut Step) { c.releases.clear(); }\n",
    }.items():
        (src / path).parent.mkdir(parents=True, exist_ok=True)
        (src / path).write_text(text)
    return src


def rows(text: str, section: str) -> list[str]:
    part = text[text.index(f"## {section}") :]
    part = part[: part.index("\n## ", 4)] if "\n## " in part[4:] else part
    return [line for line in part.split("\n") if line.startswith("| ") and not line.startswith("| ---")][1:]


class Parsing(unittest.TestCase):
    def test_a_field_s_type_may_start_on_the_next_line_and_span_lines(self):
        fields = ef.fields_of(CONTEXT)
        self.assertEqual(
            [(n, t) for n, t, _ in fields["ObjectOutputs"]],
            [("released", "Vec<(i32, Releaser)>"), ("completions", "Vec<(i32, Option<StackUnit>, bool)>")],
        )
        self.assertEqual([n for n, _, _ in fields["Step"]], ["tick", "releases", "completions"])

    def test_an_enum_s_variants_keep_their_docs_and_skip_their_fields(self):
        variants = ef.variants_of(UNIT_AI, "Effect")
        self.assertEqual([n for n, _ in variants], ["Stop", "Move", "MoveToObject"])
        self.assertEqual(variants[1][1], ["The AI's move (slot 0x130), which the unit takes."])

    def test_a_doc_s_game_routines_are_named_once_in_order(self):
        doc = ["The stop (slot 0x3c0, `FUN_14108c160`), then `FUN_14108c160` and 0x140a7a230."]
        self.assertEqual(ef.game_refs(doc), "`slot 0x3c0`, `FUN_14108c160`, `0x140a7a230`")


class Inventory(unittest.TestCase):
    def test_each_world_output_names_its_step_field_its_emitters_and_its_appliers(self):
        with tempfile.TemporaryDirectory() as d:
            text = ef.inventory(crate(Path(d)), "SRC")
        out = rows(text, "The world's outputs")
        self.assertEqual(len(out), 2)
        self.assertIn("| `releases` | `Vec<(i32, Releaser)>` | The targets the update let go", out[0])
        self.assertIn("`slot 0x180`, `FUN_140a7a230`", out[0])
        # the emitter, not the test file nor the step itself
        self.assertIn("[`construction/build.rs`](SRC/construction/build.rs) | `apply_releases` |", out[0])
        self.assertNotIn("tests.rs", out[0])
        self.assertIn("`apply_completions`", out[1])

    def test_each_unit_ai_effect_names_the_files_that_name_it(self):
        with tempfile.TemporaryDirectory() as d:
            text = ef.inventory(crate(Path(d)), "SRC")
        out = rows(text, "The unit AI's effects")
        self.assertEqual([r.split(" | ")[0] for r in out], ["| `Stop`", "| `Move`", "| `MoveToObject`"])
        self.assertTrue(out[0].endswith("[`construction/build.rs`](SRC/construction/build.rs) |"))
        self.assertTrue(out[1].endswith("[`construction/unit.rs`](SRC/construction/unit.rs) |"))

    def test_a_hand_off_is_a_field_its_doc_gives_the_caller(self):
        with tempfile.TemporaryDirectory() as d:
            text = ef.inventory(crate(Path(d)), "SRC")
        out = rows(text, "The hand-offs")
        self.assertEqual(len(out), 1)
        self.assertIn("`BuildStep` | `occupants` |", out[0])
        self.assertIn("`FUN_14123bb20`", out[0])
        self.assertTrue(out[0].endswith("[`construction/unit.rs`](SRC/construction/unit.rs) |"))
        self.assertIn("2 world outputs, 3 unit AI effects and 1 hand-offs", text)


class Check(unittest.TestCase):
    def test_the_check_fails_on_a_stale_file_and_passes_once_written(self):
        with tempfile.TemporaryDirectory() as d:
            src = crate(Path(d))
            out = Path(d) / "docs" / "EFFECTS.md"
            out.parent.mkdir()
            with redirect_stderr(io.StringIO()) as err:
                self.assertEqual(ef.main([str(src), "--out", str(out), "--check"]), 1)
            self.assertIn("not current", err.getvalue())
            with redirect_stdout(io.StringIO()):
                self.assertEqual(ef.main([str(src), "--out", str(out)]), 0)
            self.assertEqual(ef.main([str(src), "--out", str(out), "--check"]), 0)
            (src / "world" / "step.rs").write_text(CONTEXT.replace("    /// The tick.\n", "    /// The world's tick.\n"))
            self.assertEqual(ef.main([str(src), "--out", str(out), "--check"]), 0)
            (src / "construction" / "build.rs").write_text(BUILD + "fn more(c: &mut Step) { let _ = Effect::MoveToObject { target: 1 }; }\n")
            with redirect_stderr(io.StringIO()):
                self.assertEqual(ef.main([str(src), "--out", str(out), "--check"]), 1)

    def test_without_one_source_it_prints_its_usage(self):
        with redirect_stderr(io.StringIO()) as err:
            self.assertEqual(ef.main([]), 2)
        self.assertIn("scripts/effects.py SRC", err.getvalue())


if __name__ == "__main__":
    unittest.main()
