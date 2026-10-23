//! The Hunt action (`TRIBE_Action_Hunt`, type 110, vtable `0x142ed70f8`): a villager sent to a
//! tree (or an animal) with a gather task attacks it first (state 12, an [`Attack`] sub-action);
//! once it is dead the villager walks to it (state 4, a `MoveTo` sub-action), faces it (6) and
//! gathers (7). The drop-off (8 on) is not modelled yet.
//!
//! 158041: constructor `FUN_140ac2dd0`, start (slot 5) `0x140ac49b0`, update (slot 6)
//! `0x140ac7a00`, `set_state` (slot 16) `0x140ac60e0`.

use crate::attack::{Arms, Attack, Engagement, Strike, set_sprite};
use crate::damage::Victim;
use crate::move_to::{
    Goal, MoveTo, MoveWait, Walk, start_walk_or_give_up, target_moved, walk_or_give_up,
};
use crate::movement::{Mover, footprint_gap, footprint_gap_squared};
use crate::sprite::Graphic;
use crate::unit_ai::Unsupported;

/// The Hunt action's type, which word 9 adds.
pub const HUNT: i16 = 110;

/// The states the slice models: 4 walking to a dead target, 6 facing it, 7 gathering from it, 8
/// taking the load to its drop site, 12 attacking it (`0xc`).
pub const WALK: u8 = 4;
/// State 6: facing the dead target.
pub const FACE: u8 = 6;
/// State 7: gathering from it.
pub const GATHER: u8 = 7;
/// State 8: taking the load to its drop site.
pub const DROP: u8 = 8;
/// State 10 (`0xa`): leading its own livestock to the drop site, it waits for the animal to stop
/// there.
pub const LEAD: u8 = 10;
/// State 12 (`0xc`): attacking it.
pub const ATTACK: u8 = 12;
/// State 13: the hunt failed; its next update tells its unit AI (notify 0x1f9) and ends it.
pub const FAILED: u8 = 13;
/// State 2: the hunt is done with its target; it waits (a tree's target class would search
/// again, `set_state` 3).
pub const IDLE: u8 = 2;
/// State 3: searching: it told its unit AI to look for the next target (notify 0x202), again
/// every 10 seconds.
pub const SEARCH: u8 = 3;
/// State 15 (`0xf`): walking to a point a move gave it (its slot 0x40).
pub const WALK_POINT: u8 = 15;
/// State 11 (`0xb`): walking to the point after the target went on the way to it; its end
/// searches (`set_state` 3).
pub const WALK_SEARCH: u8 = 11;
/// The class of trees, for which the civilian unit AI's slot 0x1b0 (`FUN_140c1c4f0`) gives 0: a
/// hunt whose target went searches for the next one (`FUN_140ac4670`).
pub const TREE_CLASS: i32 = 15;
/// A building whose top action is this (a farm at work) takes a hunt sent to it to `set_state` 15.
const FARM_AT_WORK: i16 = 0x7d;

/// A `MoveTo` starts in place within this gap of its target (`0x140aa2100`).
const ARRIVED: f32 = 0.01;
/// Own livestock farther than this squared from its drop site is led there first
/// (`DAT_142ecccc4`).
const HERD_REACH: f32 = crate::image::hunt::HERD_REACH;
/// Own livestock this far squared from its drop site or farther is slaughtered where it stands
/// (`DAT_142eb0f64`).
const HERD_FAR: f32 = 900.0;
/// A herder within this squared gap of the livestock it leads waits for it again
/// (`DAT_142ed7180`).
const LEAD_REACH: f32 = 2.25;
/// The wait of a herder leading livestock (`set_state(10)`'s timer, `0xbf800000`).
const LEAD_WAIT: f32 = -1.0;
/// A face on dead livestock beyond the task's range plus this walks again (`DAT_142eb5690`).
const GAP: f32 = crate::image::hunt::GAP;
/// Below this a target's or a load's remainder counts as nothing (`DAT_142eb5690`).
const REMAINDER: f32 = crate::image::hunt::REMAINDER;
/// A target's last bit adds its master's first storage amount times this (`DAT_142eb1e9c`).
const LAST_BIT: f32 = crate::image::hunt::LAST_BIT;
/// A task whose `attribute_type4` is this (`0x124`) stops its target's decay under player
/// attribute 292 (`FUN_140ac44b0`).
const DECAY_TASK: i16 = 0x124;

/// A building's master type (`'P'`).
const BUILDING: u8 = 80;
/// The range of the walk to the hunter's own player's livestock (`DAT_142eb4838`).
const OWN_LIVESTOCK_RANGE: f32 = crate::image::hunt::OWN_LIVESTOCK_RANGE;
/// The villagers' class, whose `MoveTo` waits once when the motion step gives up.
const VILLAGER_CLASS: i16 = 4;
/// Classes the hunt treats apart: livestock (58, `-0xe72`) and 49 (`-0xe7b`).
pub const LIVESTOCK: i16 = 58;
/// The predators' class, whose hunt ends on its own terms (`-0xea2` raw, `0x140ac85fd`).
const PREDATOR: i16 = 10;
const CLASS_49: i16 = 49;

/// The Hunt action of a gatherer (`TRIBE_Action_Hunt`, type 110).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HuntAction {
    /// The target object (`+0x20`, its id at `+0x30`).
    pub target: i32,
    /// The action state (`+0x18`), which word 9 adds.
    pub state: u8,
    /// `+0x38`..`+0x40`: where the target stood when the hunt took it.
    pub point: [f32; 3],
    /// `+0x68`: the target's class.
    pub target_class: i16,
    /// `+0x48`: its task.
    pub task: HuntTask,
    /// The sub-action of state 12.
    pub attack: Option<Attack>,
    /// The sub-action of state 4: a `MoveTo` to the target within `walk_range`, quiet (its
    /// `+0x60` and `+0x61` set: its end changes neither the sprite nor the unit's notifies).
    pub move_to: Option<MoveTo>,
    /// The `MoveTo`'s range: the task's (`+0x30`) to a dead target, `OWN_LIVESTOCK_RANGE` to its
    /// own player's livestock.
    pub walk_range: f32,
    /// `+0x70`: the drop site its own livestock is led to, −1 for none yet.
    pub drop_site: i32,
    /// `+0x28` (its id at `+0x34`), the second target: the drop site the load goes to in state 8,
    /// −1 for none.
    pub carry_to: i32,
    /// `+0x44`: the state's timer (state 8 waits with it below 0 before it tries again).
    pub timer: f32,
    /// Its `MoveTo`'s wait after the motion step gave up.
    pub move_wait: MoveWait,
    /// What it keeps of the livestock it leads to its drop site.
    pub lead: Lead,
}

/// What a hunt keeps of the livestock it leads to its drop site (`TRIBE_Action_Hunt`'s
/// `moved_sheep` `+0x6c`, `sheep_still_time` `+0x74` and `sheep_start_x`, `_y` `+0x78`, `+0x7c`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Lead {
    /// `moved_sheep`: the hunt sent its livestock off once.
    pub moved: bool,
    /// `sheep_still_time`, which `set_state(10)` clears.
    pub still_time: i16,
    /// `sheep_start_x`, `_y`: where the animal stood when the hunt last looked.
    pub start: [f32; 2],
}

impl Lead {
    /// The constructor's (`FUN_140ac2dd0`): nothing moved or still, the start at (−1, −1)
    /// (`0xbf800000`).
    pub const NEW: Self = Self {
        moved: false,
        still_time: 0,
        start: [-1.0, -1.0],
    };
}

/// What the drop-off reads of its drop site on arrival: that the site takes what the unit carries
/// (`FUN_140fdd190`) and the gap between the two footprints squared (`FUN_14118f8c0`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropOff {
    /// The site takes what the unit carries (`FUN_140fdd190`).
    pub takes: bool,
    /// The gap between the two footprints squared (`FUN_14118f8c0`).
    pub gap_squared: f32,
}

/// What `set_state(6)` reads of a live livestock target (`0x140ac60e0` case 6).
#[derive(Clone, Copy, Debug)]
pub struct Herd {
    /// The target's player is the hunter's.
    pub own: bool,
    /// For its own livestock, the drop site (the hunt's, else the nearest that takes livestock,
    /// `FUN_1411671e0`) and the gap between it and the target squared (`FUN_14118f8c0`).
    pub drop_site: Option<(i32, f32)>,
    /// The Attack `set_state(12)` starts.
    pub engagement: Engagement,
    /// The Attack's sprites and frames.
    pub arms: Arms,
    /// The target's top action has neither a target nor a point (its slots 0x190 and 0x288,
    /// `FUN_14106d090`, `FUN_141082dc0`): it stands where it was sent.
    pub idle: bool,
    /// The hunter is of type 70 and its master has a special graphic (`+0x8e8`): the sprite
    /// leading shows.
    pub special_graphic: Option<Graphic>,
}

/// What the hunt reads of its task (`+0x48`): its id in the master's list (`+0xa`; the work command
/// keeps a hunt on the task it finds), the work range (`+0x30`) and the resource it gathers
/// (`attribute_type`, `+0x20`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HuntTask {
    /// Its id in the master's task list (`+0xa`).
    pub id: i16,
    /// The work range (`+0x30`).
    pub range: f32,
    /// The resource it gathers (`attribute_type`, `+0x20`).
    pub resource: i16,
    /// Its moving, proceeding, working and carrying sprites (`+0x40`..`+0x58`).
    pub sprites: [i16; 4],
    /// `+0x28`, `work_val1`: what one work gathers per second and work rate.
    pub work_value: f32,
    /// `+0x22`, `attribute_type2`: the player's attribute that scales the gathering, -1 none.
    pub productivity: i16,
    /// `+0x26`, `attribute_type4`.
    pub kind4: i16,
}

/// `FUN_140ac5210`, the sprite a hunter stands with: the task's carrying sprite while it holds a
/// whole unit or more of the task's resource and the sprite exists, else its master's standing
/// sprite (`standing`).
pub(crate) fn stand_sprite(
    task: &HuntTask,
    standing: i16,
    held_kind: i16,
    held: f32,
    graphics: &std::collections::BTreeMap<i16, Graphic>,
) -> Option<Graphic> {
    let g = |id: i16| (0 <= id).then(|| graphics.get(&id).copied()).flatten();
    let carrying = task.sprites[3];
    if task.resource != -1
        && task.resource == held_kind
        && 1 <= held as i32
        && g(carrying).is_some()
    {
        g(carrying)
    } else {
        g(standing)
    }
}

/// Slot 0x78 (`FUN_140a78060`, the base action's; `UNIT_ORDER`'s drop-off) of an action whose
/// task takes `resource` (`+0x20`, any with −1) for a unit carrying some of `held_kind` (`+0x310`
/// above 0, the caller's test): a load of the task's resource (compared by [`resource_kind`])
/// asks the caller for `set_state(8)` (true). A load of another resource would post notify 0x21c
/// to the unit AI, which is not modelled.
pub(crate) fn asked_drop_off(
    resource: i16,
    held_kind: i16,
    attributes: usize,
) -> Result<bool, Unsupported> {
    if resource != -1 && resource_kind(held_kind, attributes) != resource_kind(resource, attributes)
    {
        return Err(Unsupported("a drop-off of another resource (notify 0x21c)"));
    }
    Ok(true)
}

/// The player's resource of what a unit carries (`FUN_140fdd4f0`): the kinds 0 to 3, 9 and 56
/// themselves, the foods 15 to 17 food (0), any other none (−1); then 0 for a kind beyond the
/// player's `attributes` (`FUN_140efe5b0`).
pub(crate) fn resource_kind(kind: i16, attributes: usize) -> i16 {
    let kind = match kind {
        0..=3 | 9 | 56 => kind,
        15..=17 => 0,
        _ => -1,
    };
    if (0..attributes as i16).contains(&kind) {
        kind
    } else {
        0
    }
}

/// The hunt's sprites as graphics: the task's moving, working and carrying sprites, and the
/// master's standing sprite, which slot 0x68 takes for none.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HuntSprites {
    /// The moving sprite.
    pub moving: Option<Graphic>,
    /// The working sprite.
    pub working: Option<Graphic>,
    /// The carrying sprite.
    pub carrying: Option<Graphic>,
    /// The master's standing sprite.
    pub standing: Option<Graphic>,
}

impl HuntTask {
    /// [`walk_sprite`]'s id from the task's sprites (`FUN_140ac51d0`), which the hunt's Attack
    /// takes for its `MoveTo` (`FUN_140a7c210`'s argument 4).
    #[must_use]
    pub const fn walk_sprite_id(&self, held_kind: i16, held: f32) -> i16 {
        let carrying = self.resource != -1 && held_kind == self.resource && 1 <= held as i32;
        if carrying && 0 <= self.sprites[3] {
            self.sprites[3]
        } else {
            self.sprites[0]
        }
    }
}

/// `FUN_140ac51d0`: the sprite a hunt's `MoveTo` shows (its `set_state(4)`, slot 0x68 with
/// `+0x58`): the task's carrying sprite while the unit carries at least 1 of the task's
/// resource, else its moving one; None for none.
#[must_use]
pub const fn walk_sprite(
    resource: i16,
    held_kind: i16,
    held: f32,
    sprites: &HuntSprites,
) -> Option<Graphic> {
    let carrying = resource != -1 && held_kind == resource && 1 <= held as i32;
    match (carrying, sprites.carrying) {
        (true, Some(g)) => Some(g),
        _ => sprites.moving,
    }
}

/// The unit's sprite, the Sprite stream and the sprite a walk shows: a `MoveTo` that turns to
/// state 4 shows its sprite (`0x140aa29e0`, slot 0x68 with `+0x58`).
#[derive(Debug)]
pub struct Shown<'a> {
    /// The unit's sprite.
    pub sprite: &'a mut Option<crate::sprite::ActiveSprite>,
    /// The Sprite stream.
    pub rng: &'a mut crate::rng::Well512,
    /// The `MoveTo`'s sprite.
    pub graphic: Option<Graphic>,
}

impl Shown<'_> {
    /// The `MoveTo` that started turned to state 4: the unit shows its sprite.
    fn walking(&mut self, state: MoveTo) {
        if state == MoveTo::Moving
            && let Some(g) = self.graphic
        {
            set_sprite(self.sprite, g, self.rng);
        }
    }
}

/// What the gathering reads of its unit and changes besides the attack's [`Strike`].
#[derive(Debug)]
pub struct Gatherer<'a> {
    /// `+0x308`, `+0x310`: what the unit carries.
    pub held_kind: &'a mut i16,
    /// `+0x310`: how much.
    pub held: &'a mut f32,
    /// The master's work rate (`+0x798`) plus the unit's powerup (slot 0x6e8, `+0xa30`).
    pub work_rate: f32,
    /// The master's `attribute_max_amount` (`+0x61c`): what the unit carries at most.
    pub capacity: f32,
    /// The player's attribute the task's `attribute_type2` names, when above 0; else 1.
    pub productivity: f32,
    /// The target's master's class and its first storage amount (`+0x610`).
    pub target_class: i16,
    /// The target's master's first storage amount (`+0x610`).
    pub target_storage: f32,
    /// The player's attribute 292 is 1: a task whose `attribute_type4` is 0x124 stops the
    /// target's decay (`FUN_140ac44b0`).
    pub stops_decay: bool,
    /// The unit AI's target class (`+0x4c`, `FUN_140c4cc50`), None for a unit without one.
    pub ai_class: Option<i32>,
    /// The caller ran the search for the next tree a lost target asks ([`HuntAction::update`]'s
    /// `FUN_140ac4670`, `Unit::hunt_tree_search`) and found none.
    pub tree_searched: bool,
    /// The game's setting [`crate::turn::Turns::predator_hunts_end`].
    pub predator_hunts_end: bool,
    /// The sprites the hunt shows.
    pub sprites: HuntSprites,
    /// Out: the hunt let its target go and asks for `set_state(3)`'s search (the caller's: the
    /// drop site, the notify, the sprite, the timer).
    pub searches: bool,
    /// Out: the hunt sends its livestock to this drop site (the target's slot 0x138) with the
    /// herd around it (`FUN_140ac4c60`), the caller's.
    pub leads: Option<i32>,
    /// Out: the Attack's `MoveTo` failed (code 3 in state 12): `FUN_140ac4670` with −1 and 5
    /// takes the next tree for a tree's class (the caller's, `Unit::hunt_tree_search`), else (or
    /// with none found) `set_state(3)` with the target kept (the caller's search).
    pub attack_failed: bool,
    /// Out: the target the hunt let go (slot 0x68 with none), which the caller releases.
    pub let_go: Option<i32>,
}

/// What a drop-off's update did ([`HuntAction::update_drop`],
/// [`crate::gather::GatherAction::update_drop`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dropped {
    /// It walks or waits.
    Going,
    /// The load went to the player: its resource and amount, which the caller adds.
    Load(i16, f32),
    /// Its wait ended: it tries again (`set_state(8)`, the caller's).
    Again,
    /// The drop site does not take the load: the action idles (`set_state(2)`; the caller lets
    /// the targets go).
    Idle,
}

/// The wait of a hunt's drop-off that cannot go on (`0xc0400000`).
const DROP_WAIT: f32 = -3.0;
/// A hunt's drop-off reaches its drop site within this squared footprint gap (`DAT_14330eb8c`).
const DROP_REACH: f32 = crate::image::hunt::DROP_REACH;

/// Who walks to a hunt's point ([`HuntAction::update_point_walk`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointWalker {
    /// The owner is a computer player (`FUN_141659cf0`).
    pub computer: bool,
    /// The unit is a villager (class 4): its `MoveTo` waits once when the motion step gives up.
    pub villager: bool,
    /// Its unit AI's stance (`+0x1a9`), 0 without one: the wait is a second below 2, else two.
    pub stance: u8,
}

/// What a hunt's walk to its point did in an update ([`HuntAction::update_point_walk`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointWalk {
    /// It walks on, or waits to ask again.
    Walking,
    /// State 15's walk ended or failed: the hunt is done ([`HuntAction::set_done`]), its target
    /// and second target to release.
    Done {
        /// The target it let go, −1 for none.
        target: i32,
        /// The second target it let go, −1 for none.
        second: i32,
    },
    /// State 11's walk ended or failed: the hunt searches (`set_state(3)`; the caller tells the
    /// unit AI).
    Search,
}

/// What the hunt reads of its target when it starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quarry {
    /// The target.
    pub id: i32,
    /// Its place.
    pub position: [f32; 3],
    /// Its master's class and type, its state (`+0x640`) and owner.
    pub class: i16,
    /// Its master's type.
    pub kind: u8,
    /// Its state (`+0x640`).
    pub state: u8,
    /// Its owner.
    pub player: usize,
    /// What it holds (`+0x310`).
    pub amount: f32,
    /// The hunter's player counts the target's player as its own or allied (`FUN_141663d90` with
    /// the ally flag).
    pub allied: bool,
    /// Its top action's type (a building's `+0x880` list).
    pub top_action: Option<i16>,
    /// The hunter's master has its master (a building annex's main building's, `+0xbb8`) among
    /// its drop sites (`FUN_14109d4f0`), which the start reads of its own player's object.
    pub drop_site: bool,
}

/// Food and the food kinds of carcasses and fish (0, 15 to 17): a hunter keeps what it carries
/// of one for another.
fn food(kind: i16) -> bool {
    kind == 0 || (15..18).contains(&kind)
}

impl HuntAction {
    /// The factory's constructor (`FUN_140ac2dd0`; its target's add, slot 0x178, is the caller's)
    /// and the install's start (slot 5 with 1, `0x140ac49b0`): a hunter carrying another food
    /// keeps the amount as the task's kind (`FUN_14119d310` on `held`); a loaded hunter sent to
    /// one of its own drop sites carries the load there (`set_state` 8); else `set_state(4)`. A
    /// target of another, unallied player (or of class 49) that is not dead yet is attacked first
    /// (`set_state` 12). `computer` is the owner's type test (`FUN_141659cf0`); `carry_rule` the
    /// game settings' test `FUN_141302150`, with which a hunter drops another resource it carries
    /// (a computer player's does when it carries none).
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        task: HuntTask,
        target: &Quarry,
        player: usize,
        computer: bool,
        carry_rule: bool,
        held: &mut (i16, f32),
        attack: &Engagement,
        arms: Arms,
        mover: &mut Mover,
    ) -> Result<(Self, bool), Unsupported> {
        let mut hunt = Self {
            target: target.id,
            state: 0,
            point: target.position,
            target_class: target.class,
            task,
            attack: None,
            move_to: None,
            walk_range: task.range,
            drop_site: -1,
            carry_to: -1,
            timer: 0.0,
            move_wait: MoveWait::default(),
            lead: Lead::NEW,
        };
        if 0.0 < target.amount {
            let kind = task.resource;
            let both_food = food(kind) && food(held.0);
            let drops = computer && held.1 == 0.0;
            if held.0 != kind {
                if both_food {
                    held.0 = kind;
                } else if drops || carry_rule {
                    *held = (kind, 0.0);
                }
            }
        }
        // the start (slot 5 with 1): an own target among the hunter's master's drop sites
        // (`+0x7a0`, FUN_14109d4f0; a building annex's main building) takes a load there: it
        // becomes the second target (FUN_140a7a190), the target goes (slot 0x68 with none; the
        // caller takes and releases it), the point and `+0x68` clear, `set_state(8)`; a hunter
        // carrying nothing does not get there
        if target.player == player && target.drop_site && 0.0 < held.1 {
            hunt.target = -1;
            hunt.point = [-1.0; 3];
            hunt.target_class = -1;
            hunt.carry_to = target.id;
            hunt.set_drop(attack.gap, mover)?;
            return Ok((hunt, false));
        }
        let arrived = hunt.chase(target, player, attack, arms, mover)?;
        Ok((hunt, arrived))
    }

    /// `set_state(4)` (`0x140ac60e0` case 4): the sub-actions go (an Attack's `MoveTo` stops the
    /// unit), the timer clears and the target's place becomes the point. A dead target, or the hunter's own
    /// player's livestock, is walked to: a quiet `MoveTo` within the task's range, or
    /// `OWN_LIVESTOCK_RANGE` to the livestock (`FUN_140aa0df0`; its start `0x140aa2100`: a unit
    /// within 0.01 of the target's footprint is there already). A target not dead yet of another,
    /// unallied player (or of class 49) is attacked first: `set_state(12)` (case 0xc) with an
    /// Attack of the master's ranges. A live allied one fails the hunt (`set_state` 13).
    pub(crate) fn chase(
        &mut self,
        target: &Quarry,
        player: usize,
        attack: &Engagement,
        arms: Arms,
        mover: &mut Mover,
    ) -> Result<bool, Unsupported> {
        self.tear_down(Some(mover))?;
        self.state = 4;
        self.timer = 0.0;
        self.point = target.position;
        let own_livestock = target.class == LIVESTOCK && target.player == player;
        if 2 < target.state || own_livestock {
            self.move_wait = MoveWait::default();
            self.walk_range = if 2 < target.state {
                self.task.range
            } else {
                OWN_LIVESTOCK_RANGE
            };
            self.move_to = Some(if ARRIVED < attack.gap {
                if mover.stats.speed == 0.0 {
                    return Err(Unsupported("a MoveTo of a unit without speed (state 13)"));
                }
                MoveTo::Start
            } else {
                MoveTo::Arrived
            });
            return Ok(false);
        }
        if target.class != CLASS_49 && target.allied {
            // LAB_140ac679d: set_state(13): the timer clears; the unit's sprite (slot 0x68 with
            // FUN_140ac5210's) is the caller's
            self.state = FAILED;
            self.timer = 0.0;
            return Ok(false);
        }
        self.state = ATTACK;
        let (a, arrived) = Attack::start(target.id, attack, arms, mover)?;
        self.attack = Some(a);
        Ok(arrived)
    }

    /// Slot 9 (`0x140ac97c0`), the work command's new target for the hunt it keeps: true when the
    /// hunt takes it. A building at work as a farm (top action `0x7d`) takes it to `set_state` 15,
    /// not modelled. Its own player's drop site (one of its master's, `drops_here`,
    /// `FUN_14109d4f0`) while it carries something (`held`) takes the load there: the target goes
    /// (slot 0x68 with none; the caller releases it), the timer and the point clear, the drop site
    /// becomes the second target (`FUN_140a7a190`) with its class, and `set_state(8)`. Another target
    /// becomes the hunt's (slot 0x68, `FUN_140ac7980`; the caller releases the old target and takes
    /// the new) and the hunt starts over at `set_state(4)`. Its own target changes nothing. Some
    /// when it took the target, true when its Attack's `MoveTo` starts arrived (the caller shows
    /// the idle sprite, [`crate::attack::show_arrived`]).
    #[allow(clippy::too_many_arguments)]
    pub fn retarget(
        &mut self,
        target: &Quarry,
        player: usize,
        held: f32,
        drops_here: bool,
        attack: &Engagement,
        arms: Arms,
        mover: &mut Mover,
    ) -> Result<Option<bool>, Unsupported> {
        if target.kind == BUILDING && target.top_action == Some(FARM_AT_WORK) {
            return Err(Unsupported("a hunt sent to a farm at work (set_state 15)"));
        }
        if target.player == player && drops_here && 0.0 < held {
            self.target = -1;
            self.timer = 0.0;
            self.point = [-1.0; 3];
            self.carry_to = target.id;
            self.target_class = target.class;
            self.set_drop(attack.gap, mover)?;
            return Ok(Some(false));
        }
        if target.id == self.target {
            return Ok(None);
        }
        self.target = target.id;
        self.target_class = target.class;
        self.carry_to = -1;
        self.chase(target, player, attack, arms, mover).map(Some)
    }

    /// Slot 0x40 (`FUN_140ac52c0`): the hunt takes a move's `target` (None for a point) and
    /// `point`; true when it does. A point with a task of a work range it does not take. A unit
    /// on a pasture (`mMyPastureId` +0x794) of another target would leave it (`FUN_141189ca0`),
    /// not modelled. Its own player's drop site (`drops_here`) while it carries something
    /// (`held`) takes the load there: unless it walks there already (state 8), the target goes
    /// (slot 0x68 with none; the caller releases it), the point clears, `+0x68` takes the drop
    /// site's class, which becomes the second target (`FUN_140a7a190`; the caller takes it) and
    /// `set_state(8)`. Any other target, or a point: the point is the move's, `+0x68` −1, the
    /// target goes and `set_state(15)`: a quiet `MoveTo` to the point within the task's range
    /// (`FUN_140aa0d30`).
    #[allow(clippy::too_many_arguments)]
    pub fn take_move(
        &mut self,
        target: Option<&Quarry>,
        point: [f32; 3],
        player: usize,
        held: f32,
        drops_here: bool,
        my_pasture: i32,
        gap: f32,
        mover: &mut Mover,
    ) -> Result<bool, Unsupported> {
        if target.is_none() && self.task.range != 0.0 {
            return Ok(false);
        }
        if my_pasture != -1 && target.is_none_or(|t| t.id != my_pasture) {
            return Err(Unsupported("a hunter leaving its pasture (FUN_141189ca0)"));
        }
        if let Some(t) = target.filter(|t| t.player == player && drops_here && 0.0 < held) {
            if t.id == self.carry_to && self.state == DROP {
                return Ok(true);
            }
            self.target = -1;
            self.point = [-1.0; 3];
            self.target_class = t.class;
            self.carry_to = t.id;
            self.set_drop(gap, mover)?;
            return Ok(true);
        }
        self.point = point;
        self.target_class = -1;
        self.target = -1;
        self.set_point_walk(WALK_POINT, mover)?;
        Ok(true)
    }

    /// `set_state(11)` after a drop-off that found no next target: the walk to the hunt's point
    /// ([`HuntAction::set_point_walk`]), whose end searches again.
    ///
    /// # Errors
    ///
    /// A unit without speed.
    pub fn search_walk(&mut self, mover: &mut Mover) -> Result<(), Unsupported> {
        self.set_point_walk(WALK_SEARCH, mover)
    }

    /// `set_state` 11 and 15 (`0x140ac60e0` case 0xb): the sub-actions go, the timer clears and a
    /// quiet `MoveTo` to the point within the task's range starts (`FUN_140aa0d30`, showing the
    /// walking sprite of [`walk_sprite`]).
    fn set_point_walk(&mut self, state: u8, mover: &mut Mover) -> Result<(), Unsupported> {
        self.tear_down(Some(mover))?;
        self.state = state;
        self.timer = 0.0;
        self.walk_range = self.task.range;
        self.move_wait = MoveWait::default();
        if mover.stats.speed == 0.0 {
            return Err(Unsupported("a MoveTo of a unit without speed (state 13)"));
        }
        self.move_to = Some(MoveTo::Start);
        Ok(())
    }

    /// State 4's wait after its `MoveTo` failed (`0x140ac7a00` case 3, its timer below 0): the
    /// timer runs by `dt`; at its end it clears and the hunt walks to its point (`set_state(11)`,
    /// the target kept), whose end searches.
    ///
    /// # Errors
    ///
    /// A unit that cannot move.
    pub fn wait_then_walk(&mut self, dt: f32, mover: &mut Mover) -> Result<(), Unsupported> {
        self.timer += dt;
        if self.timer < 0.0 {
            return Ok(());
        }
        self.timer = 0.0;
        self.set_point_walk(WALK_SEARCH, mover)
    }

    /// State 15's and 11's update (`0x140ac7a00` cases 0xe and 0xa): the `MoveTo` to the point
    /// walks; the motion step giving up turns it to its wait or its failure
    /// ([`HuntAction::give_up_walk`]); a waiting one asks again after its wait (its case 2, its
    /// update returns 0). Once it ended (its update returns 1, 2 or 5) or failed (3, quiet: no
    /// notify), the `set_state` that follows tears it down (`FUN_140a98620`; its destructor stops
    /// the unit, slot 0x5d0): a human player's hunt in state 11 searches (`set_state(3)`: the
    /// state here, the rest the caller's), one in state 15 is done (`set_state(2)`: the target
    /// goes, the point clears and the timer; the unit takes its standing sprite and the second
    /// target goes, the caller's). A computer player's ended walk in state 15 (`set_state` 1) or
    /// failed one (`set_state` 13) is not modelled; a point walk has no target to lose (code 4).
    pub fn update_point_walk(
        &mut self,
        mut walk: Walk<'_>,
        who: PointWalker,
        mut shown: Shown<'_>,
    ) -> Result<PointWalk, Unsupported> {
        let ended = match self.move_to {
            Some(MoveTo::Start) => {
                // case 3 after a wait: `+0x6c` goes from 1 to 2 as it asks again
                if self.move_wait.state == 1 {
                    self.move_wait.state = 2;
                }
                self.move_to = Some(MoveTo::Moving);
                let range = self.walk_range;
                walk.me.action_state = self.state;
                let walked = start_walk_or_give_up(walk, Goal::Point(self.point), range, true)?;
                shown.walking(walked.unwrap_or(MoveTo::Moving));
                match walked {
                    Some(state) => self.move_to = Some(state),
                    None => self.give_up_walk(who),
                }
                return Ok(PointWalk::Walking);
            }
            Some(MoveTo::Moving) => {
                walk.me.action_state = self.state;
                match walk_or_give_up(walk)? {
                    Some(state) => self.move_to = Some(state),
                    None => self.give_up_walk(who),
                }
                return Ok(PointWalk::Walking);
            }
            Some(MoveTo::Waiting) => {
                self.move_wait.timer += walk.dt;
                if self.move_wait.delay <= self.move_wait.timer {
                    self.move_wait.timer = 0.0;
                    self.move_to = Some(MoveTo::Start);
                }
                return Ok(PointWalk::Walking);
            }
            Some(MoveTo::Arrived) => true,
            Some(MoveTo::Failed) => false,
            None => return Err(Unsupported("a Hunt's walk to a point without its MoveTo")),
        };
        if who.computer {
            return Err(Unsupported(if ended {
                "a computer player's hunt after its walk (set_state 1)"
            } else {
                "a computer player's hunt whose walk to a point failed (set_state 13)"
            }));
        }
        self.move_to = None;
        walk.mover
            .as_deref_mut()
            .ok_or(Unsupported("a hunter without movement state"))?
            .stop()?;
        if self.state == WALK_SEARCH {
            self.state = SEARCH;
            return Ok(PointWalk::Search);
        }
        let [target, second] = self.set_done();
        Ok(PointWalk::Done { target, second })
    }

    /// `set_state(2)` (`0x140ac60e0` case 2): the hunt is done. The unit takes the sprite it
    /// stands with (slot 0x68 with `FUN_140ac5210`'s, the caller's), the target and the second
    /// target go (slot 0x68 and `FUN_140a7a190` with none: returned in that order for the caller
    /// to release), the point clears and the timer.
    pub const fn set_done(&mut self) -> [i32; 2] {
        self.state = IDLE;
        let target = std::mem::replace(&mut self.target, -1);
        let second = std::mem::replace(&mut self.carry_to, -1);
        self.point = [-1.0; 3];
        self.timer = 0.0;
        [target, second]
    }

    /// The point walk's `MoveTo` answer to the motion step giving up ([`MoveWait::give_up`]: a
    /// human player's villager waits once; quiet).
    pub const fn give_up_walk(&mut self, who: PointWalker) {
        let waits = !who.computer && who.villager;
        self.move_to = Some(self.move_wait.give_up(waits, who.stance));
    }

    /// `set_state(8)` (`0x140ac60e0` case 8) with a load and its drop site (`carry_to`; without one
    /// the drop site search `FUN_1411671e0` would find it, not modelled): the sub-actions go and a
    /// quiet `MoveTo` to the drop site within the task's range starts (`FUN_140aa0df0`, as
    /// [`HuntAction::set_walk`]'s: within 0.01 of its footprint (`gap`) the unit is there already).
    pub(crate) fn set_drop(&mut self, gap: f32, mover: &mut Mover) -> Result<(), Unsupported> {
        self.tear_down(Some(mover))?;
        self.state = DROP;
        if self.carry_to == -1 {
            return Err(Unsupported("a drop-off without its drop site (its search)"));
        }
        self.walk_range = self.task.range;
        self.move_wait = MoveWait::default();
        self.move_to = Some(if ARRIVED < gap {
            if mover.stats.speed == 0.0 {
                return Err(Unsupported("a MoveTo of a unit without speed (state 13)"));
            }
            MoveTo::Start
        } else {
            MoveTo::Arrived
        });
        Ok(())
    }

    /// State 8's update (`0x140ac7a00` case 7): while its timer is below 0 the hunt waits (the
    /// timer runs by dt); at its end the drop site goes (`FUN_140a7a190` with none) and the hunt
    /// tries again (`set_state(8)`, [`Dropped::Again`], the caller's). Else the `MoveTo` to the
    /// drop site walks, waits after the motion step gave up (a human player's villager once:
    /// [`HuntAction::give_up_walk`]) or fails; a failed one (codes 3 to 5) makes the hunt wait 3
    /// seconds and stops the unit's animation (`FUN_141161850`). Once it ended (its update in
    /// state 1 returns 1: the list deletes it and its destructor stops the unit) the load goes to
    /// the player at a drop site that takes it within a tile (`site`; one that does not,
    /// `set_state` 2, is not modelled; a farther one makes the hunt wait 3 seconds too): the
    /// resource of what the unit carries (`FUN_140fdd4f0`: food for 15 to 17, nothing outside 0
    /// to 3, 9 and 56, then 0 beyond the player's attributes, `FUN_140efe5b0`), which the caller
    /// adds; the unit carries nothing after (`FUN_14119d310` with -1). A hunt with a target walks
    /// back to it (`set_state` 4), one without searches for a new one (the caller's).
    #[allow(clippy::too_many_arguments)]
    pub fn update_drop(
        &mut self,
        mut walk: Walk<'_>,
        held_kind: &mut i16,
        held: &mut f32,
        attributes: usize,
        site: Option<DropOff>,
        mut shown: Shown<'_>,
        who: PointWalker,
        anim_speed: &mut Option<f32>,
    ) -> Result<Dropped, Unsupported> {
        if self.timer < 0.0 {
            self.timer += walk.dt;
            if self.timer < 0.0 {
                return Ok(Dropped::Going);
            }
            self.timer = 0.0;
            return Ok(Dropped::Again);
        }
        match self.move_to {
            Some(MoveTo::Start) => {
                self.move_to = Some(MoveTo::Moving);
                let range = self.walk_range;
                walk.me.action_state = self.state;
                let walked = start_walk_or_give_up(walk, Goal::Target(self.carry_to), range, true)?;
                shown.walking(walked.unwrap_or(MoveTo::Moving));
                match walked {
                    Some(state) => self.move_to = Some(state),
                    None => self.give_up_walk(who),
                }
                return Ok(Dropped::Going);
            }
            Some(MoveTo::Moving) => {
                if target_moved(&walk, self.carry_to, true)?.again() {
                    self.move_to = Some(MoveTo::Start);
                    return Ok(Dropped::Going);
                }
                walk.me.action_state = self.state;
                match walk_or_give_up(walk)? {
                    Some(m) => self.move_to = Some(m),
                    None => self.give_up_walk(who),
                }
                return Ok(Dropped::Going);
            }
            Some(MoveTo::Waiting) => {
                self.move_wait.timer += walk.dt;
                if self.move_wait.delay <= self.move_wait.timer {
                    self.move_wait.timer = 0.0;
                    self.move_to = Some(MoveTo::Start);
                }
                return Ok(Dropped::Going);
            }
            Some(MoveTo::Failed) => {
                self.timer = DROP_WAIT;
                *anim_speed = Some(0.0);
                return Ok(Dropped::Going);
            }
            Some(MoveTo::Arrived) => {
                self.move_to = None;
                walk.mover
                    .as_deref_mut()
                    .ok_or(Unsupported("a hunter without movement state"))?
                    .stop()?;
            }
            None => return Err(Unsupported("a Hunt in state 8 without its MoveTo")),
        }
        let site = site.ok_or(Unsupported("a drop-off whose drop site is gone"))?;
        if !site.takes {
            return Err(Unsupported(
                "a drop site that does not take the load (set_state 2)",
            ));
        }
        if DROP_REACH < site.gap_squared {
            self.timer = DROP_WAIT;
            *anim_speed = Some(0.0);
            return Ok(Dropped::Going);
        }
        let kind = resource_kind(*held_kind, attributes);
        let amount = *held;
        *held_kind = -1;
        *held = 0.0;
        Ok(Dropped::Load(kind, amount))
    }

    /// Slot 0x78 (`FUN_140a78060`, the base action's; `UNIT_ORDER`'s drop-off) of a unit carrying
    /// some of `held_kind` (`+0x310` above 0, which the caller checked): a load of the task's resource (`+0x20`, of any with −1; compared by
    /// [`resource_kind`]) asks the caller for `set_state(8)` (true). A load of another resource
    /// would post notify 0x21c to the unit AI, which is not modelled.
    pub fn drop_off_asked(&self, held_kind: i16, attributes: usize) -> Result<bool, Unsupported> {
        asked_drop_off(self.task.resource, held_kind, attributes)
    }

    /// Slot 0x68 with none (`FUN_140ac7980`): the target goes and the lead's fields clear (its
    /// start −1); the old target, which the caller releases (slot 0x180).
    pub const fn let_go(&mut self) -> i32 {
        let target = self.target;
        self.target = -1;
        self.lead = Lead {
            moved: false,
            still_time: 0,
            start: [-1.0, -1.0],
        };
        target
    }

    /// The teardown (`FUN_140a73fc0`): the sub-actions go; the caller releases the target (slot
    /// 0x180).
    pub fn tear_down(&mut self, mut mover: Option<&mut Mover>) -> Result<(), Unsupported> {
        if let Some(mut a) = self.attack.take() {
            a.tear_down(mover.as_deref_mut())?;
        }
        if self.move_to.take().is_some()
            && let Some(m) = mover
        {
            m.stop()?;
        }
        Ok(())
    }

    /// Case 6 without a target (`0x140ac7a00` at `0x140ac9029`): a load is carried off
    /// (`set_state(8)`, true, the caller's); with none, or one that is not a number (`COMISS`
    /// with `JBE`), the hunt searches (`set_state(3)`: [`SEARCH`] here, the rest the caller's).
    fn ends_without_target(&mut self, held: f32) -> bool {
        if crate::fmath::at_most_or_unordered(held, 0.0) {
            self.state = SEARCH;
            false
        } else {
            true
        }
    }

    /// One update (slot 6, `0x140ac7a00`): state 12 updates the Attack (`FUN_140a992b0`); its end
    /// on the dead target (code 5) runs `FUN_140ac44b0` and `set_state(4)`. State 4 updates the
    /// `MoveTo`; its end (code 1) runs `FUN_140ac44b0` and `set_state(6)`. State 6 faces the target
    /// (`FUN_141162db0`) and, while it holds something, `set_state(7)`; state 7 gathers. A computer
    /// player's hunt and the other codes are not modelled.
    pub fn update(
        &mut self,
        mut walk: Walk<'_>,
        computer: bool,
        strike: &mut Strike<'_>,
        gather: &mut Gatherer<'_>,
        herd: Option<Herd>,
    ) -> Result<bool, Unsupported> {
        if computer {
            return Err(Unsupported("a computer player's hunt (FUN_140fde4f0)"));
        }
        let mut full = false;
        // a target gone or past state 6 went at the update's start (slot 0x68 with none, the
        // caller's): in state 4 its `MoveTo`, quiet, forgets it too and returns 5
        if self.state == WALK && self.target == -1 && 0.0 <= self.timer && self.move_to.is_some() {
            self.lost_on_the_way(&mut walk, gather)?;
            return Ok(false);
        }
        let r = match self.state {
            ATTACK => {
                let attack = self
                    .attack
                    .as_mut()
                    .ok_or(Unsupported("a Hunt in state 12 without its Attack"))?;
                match attack.update(walk.reborrow(), strike)? {
                    0 | 6.. => Ok(()),
                    // the Attack ended (1, 2, 5): with no target left set_state(3); a predator
                    // (class 10) of a unit AI with orders queued (`FUN_140c564a0` above 0) ends
                    // the hunt (set_state 1) under the game's setting `+0x98`, not modelled;
                    // else `FUN_140ac44b0` and `set_state(4)`
                    1 | 2 | 5 if self.target == -1 => {
                        self.tear_down(walk.mover.as_deref_mut())?;
                        self.state = SEARCH;
                        gather.searches = true;
                        Ok(())
                    }
                    1 | 2 | 5
                        if gather.predator_hunts_end
                            && strike.victim.class == PREDATOR
                            && !strike.queue_empty =>
                    {
                        Err(Unsupported(
                            "a hunt that ends with its Attack on a predator (set_state 1)",
                        ))
                    }
                    1 | 2 | 5 => {
                        self.decay_check(gather.stops_decay, strike.victim);
                        let player = strike.own.player;
                        self.set_walk(&mut walk, strike.victim, player)
                    }
                    // it failed (its MoveTo did; its update in state 13 says so again each
                    // time): the search for the next tree, else set_state(3), the caller's
                    3 => {
                        gather.attack_failed = true;
                        Ok(())
                    }
                    // the target went inside another (`LAB_140ac7ecd`): it goes (slot 0x68 with
                    // none) and the hunt walks to its point to search there (set_state 11)
                    _ => {
                        gather.let_go = Some(self.let_go());
                        let mover = walk
                            .mover
                            .as_deref_mut()
                            .ok_or(Unsupported("a hunter without movement state"))?;
                        self.set_point_walk(WALK_SEARCH, mover)
                    }
                }
            }
            // case 3: a hunt that gave up on its MoveTo waits (its timer below 0), then set_state 11
            WALK if self.timer < 0.0 => {
                let mover = walk
                    .mover
                    .as_deref_mut()
                    .ok_or(Unsupported("a hunter without movement state"))?;
                self.wait_then_walk(walk.dt, mover)
            }
            WALK => match self.move_to {
                Some(MoveTo::Start) => {
                    // case 3 after a wait: `+0x6c` goes from 1 to 2 as it asks again
                    if self.move_wait.state == 1 {
                        self.move_wait.state = 2;
                    }
                    self.move_to = Some(MoveTo::Moving);
                    let range = self.walk_range;
                    walk.me.action_state = self.state;
                    let goal = Goal::Target(self.target);
                    let walked = start_walk_or_give_up(walk.reborrow(), goal, range, true)?;
                    crate::attack::in_the_way(&mut walk, strike);
                    // the MoveTo's set_state(4) shows the hunt's walking sprite
                    let shows = walk_sprite(
                        self.task.resource,
                        *gather.held_kind,
                        *gather.held,
                        &gather.sprites,
                    );
                    Shown {
                        sprite: strike.sprite,
                        rng: strike.sprite_rng,
                        graphic: shows,
                    }
                    .walking(walked.unwrap_or(MoveTo::Moving));
                    match walked {
                        Some(state) => self.move_to = Some(state),
                        None => self.give_up(strike),
                    }
                    Ok(())
                }
                Some(MoveTo::Moving) => {
                    if target_moved(&walk, self.target, true)?.again() {
                        self.move_to = Some(MoveTo::Start);
                        return Ok(false);
                    }
                    walk.me.action_state = self.state;
                    let walked = walk_or_give_up(walk.reborrow())?;
                    crate::attack::in_the_way(&mut walk, strike);
                    match walked {
                        Some(m) => self.move_to = Some(m),
                        None => self.give_up(strike),
                    }
                    Ok(())
                }
                // the failed MoveTo's update returns 3 (quiet: no notify): `FUN_140ac4670` with 8
                // and -1 looks for another target only for a class the unit AI's slot 0x1b0 gives
                // 0 (`FUN_140c1c4f0`: 15, trees; the class its unit AI targets, else the hunt's:
                // the caller's search, `Unit::hunt_tree_search`, which found none); then the hunt
                // waits 3 seconds and the unit's animation stops (`FUN_141161850`)
                Some(MoveTo::Failed) => {
                    if let Some(class) = gather.ai_class {
                        let class = if class == -1 {
                            i32::from(self.target_class)
                        } else {
                            class
                        };
                        if class == TREE_CLASS && !gather.tree_searched {
                            return Err(Unsupported(
                                "a hunt's search for another tree (FUN_140ac4670)",
                            ));
                        }
                    }
                    self.timer = -3.0;
                    *strike.anim_speed = Some(0.0);
                    Ok(())
                }
                // case 2: the timer runs by dt; at its end the MoveTo asks again (set_state 3)
                Some(MoveTo::Waiting) => {
                    self.move_wait.timer += walk.dt;
                    if self.move_wait.delay <= self.move_wait.timer {
                        self.move_wait.timer = 0.0;
                        self.move_to = Some(MoveTo::Start);
                    }
                    Ok(())
                }
                // its update in state 1 returns 1: the list deletes it, and its destructor
                // (`0x140aa1f30`) stops the unit (slot 0x5d0)
                Some(MoveTo::Arrived) => {
                    self.move_to = None;
                    walk.mover
                        .as_deref_mut()
                        .ok_or(Unsupported("a hunter without movement state"))?
                        .stop()?;
                    self.decay_check(gather.stops_decay, strike.victim);
                    self.set_face(walk, strike, gather, herd)
                }
                None => Err(Unsupported("a Hunt in state 4 without its MoveTo")),
            },
            LEAD => self.update_lead(walk, strike, gather, herd),
            FACE => {
                // case 5: FUN_141162db0 turns the unit to face its target; a unit without a turn
                // time turns at once, by the facets between
                let at = walk
                    .world
                    .get(walk.me.id)
                    .map_or(walk.me.position, |p| p.position);
                let mover = walk
                    .mover
                    .as_deref_mut()
                    .ok_or(Unsupported("a hunter without movement state"))?;
                if !crate::attack::face_target(
                    mover,
                    at,
                    strike.victim.position,
                    strike.world_time,
                )? {
                    return Err(Unsupported(
                        "a hunter that turns over time to face its target",
                    ));
                }
                if 0.0 < strike.victim.amount {
                    self.set_gather(strike, gather);
                } else {
                    // the target goes (slot 0x68 with none, the caller's) and set_state(3)
                    self.state = SEARCH;
                    gather.searches = true;
                }
                Ok(())
            }
            // case 6 without a target (or a task): set_state(8) to carry the load off, set_state(3)
            // with none
            GATHER if self.target == -1 => {
                full = self.ends_without_target(*gather.held);
                if !full {
                    gather.searches = true;
                }
                Ok(())
            }
            GATHER => {
                full = self.gather(walk.dt, strike.victim, gather)?;
                Ok(())
            }
            _ => Err(Unsupported(
                "a Hunt action in a state the slice does not model",
            )),
        };
        r.map(|()| full)
    }

    /// State 4's answer to its `MoveTo` losing the target (codes 4 and 5): `FUN_140ac4670` with 8
    /// and -1 looks for another target of a class the unit AI's slot 0x1b0 gives 0 (trees: the
    /// caller's search, `Unit::hunt_tree_search`, which found none); then the target goes (slot
    /// 0x68 with none) and the hunt walks to its point (`set_state` 11).
    fn lost_on_the_way(
        &mut self,
        walk: &mut Walk<'_>,
        gather: &Gatherer<'_>,
    ) -> Result<(), Unsupported> {
        if let Some(class) = gather.ai_class {
            let class = if class == -1 {
                i32::from(self.target_class)
            } else {
                class
            };
            if class == TREE_CLASS && !gather.tree_searched {
                return Err(Unsupported(
                    "a hunt's search for another tree (FUN_140ac4670)",
                ));
            }
        }
        let mover = walk
            .mover
            .as_deref_mut()
            .ok_or(Unsupported("a hunter without movement state"))?;
        self.set_point_walk(WALK_SEARCH, mover)
    }

    /// `set_state(12)` on live livestock: the Attack of the master's ranges starts (with its
    /// target in reach its point and the facing, [`Attack::start`]), showing the idle sprite when
    /// its `MoveTo` starts arrived.
    fn start_attack(
        &mut self,
        walk: Walk<'_>,
        strike: &mut Strike<'_>,
        herd: &Herd,
    ) -> Result<(), Unsupported> {
        let mover = walk
            .mover
            .ok_or(Unsupported("a hunter without movement state"))?;
        self.move_to = None;
        self.state = ATTACK;
        let (a, arrived) = Attack::start(self.target, &herd.engagement, herd.arms, mover)?;
        if arrived {
            a.arrived_sprite(strike)?;
        }
        self.attack = Some(a);
        Ok(())
    }

    /// `set_state(10)` (`0x140ac60e0` case 10): `sheep_still_time` clears, the animal's place
    /// becomes the start (`+0x78`, `+0x7c`), the hunter faces it (`FUN_141162db0`, at once; one
    /// that turns over time is not modelled), a hunter of type 70 with a special graphic shows
    /// it (slot 0x68 with master `+0x8e8`) and the timer starts at −1 s.
    fn set_lead(
        &mut self,
        mut walk: Walk<'_>,
        strike: &mut Strike<'_>,
        herd: &Herd,
    ) -> Result<(), Unsupported> {
        self.tear_down(walk.mover.as_deref_mut())?;
        self.state = LEAD;
        self.lead.still_time = 0;
        let at = strike.victim.position;
        self.lead.start = [at[0], at[1]];
        self.face_animal(&mut walk, strike)?;
        if let Some(g) = herd.special_graphic {
            set_sprite(strike.sprite, g, strike.sprite_rng);
        }
        self.timer = LEAD_WAIT;
        Ok(())
    }

    /// `FUN_141162db0` on the target at once (a unit without a turn time turns by the facets
    /// between).
    fn face_animal(&self, walk: &mut Walk<'_>, strike: &Strike<'_>) -> Result<(), Unsupported> {
        let at = walk
            .world
            .get(walk.me.id)
            .map_or(walk.me.position, |p| p.position);
        let mover = walk
            .mover
            .as_deref_mut()
            .ok_or(Unsupported("a hunter without movement state"))?;
        if !crate::attack::face_target(mover, at, strike.victim.position, strike.world_time)? {
            return Err(Unsupported(
                "a hunter that turns over time to face its target",
            ));
        }
        Ok(())
    }

    /// State 10's update (`0x140ac7a00` case 9): without its target the hunt searches
    /// (`set_state(3)`, the caller's). An animal that stands where it was sent (`Herd::idle`) is
    /// slaughtered (`set_state(12)`); else the timer runs by dt and at its end an animal on the
    /// start is slaughtered too; one that moved but stays within 1.5 tiles (`LEAD_REACH`
    /// squared, `FUN_14118f8c0`) becomes the start again, the hunter faces it and waits another
    /// second; one farther is walked to (`set_state(4)`).
    fn update_lead(
        &mut self,
        mut walk: Walk<'_>,
        strike: &mut Strike<'_>,
        gather: &mut Gatherer<'_>,
        herd: Option<Herd>,
    ) -> Result<(), Unsupported> {
        if self.target == -1 {
            self.state = SEARCH;
            gather.searches = true;
            return Ok(());
        }
        let herd = herd.ok_or(Unsupported("a hunt leading livestock unprepared"))?;
        if herd.idle {
            return self.start_attack(walk, strike, &herd);
        }
        self.timer += walk.dt;
        if self.timer < 0.0 {
            return Ok(());
        }
        let v = strike.victim.position;
        if v[0] == self.lead.start[0] && v[1] == self.lead.start[1] {
            return self.start_attack(walk, strike, &herd);
        }
        let at = walk
            .world
            .get(walk.me.id)
            .map_or(walk.me.position, |p| p.position);
        let ground = walk
            .mover
            .as_deref()
            .map(|m| [m.stats.radius[0], m.stats.radius[1]])
            .ok_or(Unsupported("a hunter without movement state"))?;
        if footprint_gap_squared(at, ground, v, strike.victim.radius) <= LEAD_REACH {
            self.lead.start = [v[0], v[1]];
            self.face_animal(&mut walk, strike)?;
            self.timer = LEAD_WAIT;
            return Ok(());
        }
        let player = strike.own.player;
        self.set_walk(&mut walk, strike.victim, player)
    }

    /// `FUN_140ac44b0`: a task whose `attribute_type4` is 0x124, with its player's attribute 292
    /// at 1, stops its target's decay: a target whose master decays (`+0x620` above 0) gets a
    /// master of its own unless it has one (`FUN_141188bc0`, `FUN_141186d10`) and that master's
    /// decay takes the other sign (its bits XOR `0x80000000`); the caller writes it back to the
    /// target ([`Victim::decay`]). Otherwise it does nothing.
    fn decay_check(&self, stops_decay: bool, victim: &mut Victim) {
        if self.task.kind4 == DECAY_TASK && stops_decay && 0.0 < victim.decay {
            victim.decay = f32::from_bits(victim.decay.to_bits() ^ 0x8000_0000);
        }
    }

    /// The `MoveTo`'s answer to the motion step giving up ([`MoveWait::give_up`]; a stance below 2
    /// without a unit AI; quiet).
    const fn give_up(&mut self, strike: &Strike<'_>) {
        let waits = !strike.computer && strike.own.class == VILLAGER_CLASS;
        self.move_to = Some(self.move_wait.give_up(waits, strike.stance));
    }

    /// `set_state(4)` (`0x140ac60e0` case 4) after the target died, or for live livestock of the
    /// hunter's `player`: the sub-actions go, the timer clears, the target's place becomes the
    /// point, and a quiet `MoveTo` to the target within the task's range (`OWN_LIVESTOCK_RANGE`
    /// to own livestock) starts (`FUN_140aa0df0`, its start `0x140aa2100`: a unit within 0.01 of
    /// the target's footprint is there already, state 1; else state 3). Another live target is
    /// not modelled.
    fn set_walk(
        &mut self,
        walk: &mut Walk<'_>,
        victim: &Victim,
        player: usize,
    ) -> Result<(), Unsupported> {
        self.tear_down(walk.mover.as_deref_mut())?;
        self.state = WALK;
        self.timer = 0.0;
        let own_livestock = victim.class == LIVESTOCK && victim.player == player;
        if victim.state <= 2 && !own_livestock {
            return Err(Unsupported(
                "a hunt's set_state 4 on a live target after its attack",
            ));
        }
        let t = walk
            .world
            .get_known(self.target)?
            .ok_or(Unsupported("a hunt's target without a place"))?;
        self.point = t.position;
        self.walk_range = if victim.state <= 2 {
            OWN_LIVESTOCK_RANGE
        } else {
            self.task.range
        };
        self.move_wait = MoveWait::default();
        let mover = walk
            .mover
            .as_deref()
            .ok_or(Unsupported("a hunter without movement state"))?;
        let ground = [mover.stats.radius[0], mover.stats.radius[1]];
        let gap = footprint_gap(walk.me.position, ground, t.position, t.radius);
        self.move_to = Some(if ARRIVED < gap {
            if mover.stats.speed == 0.0 {
                return Err(Unsupported("a MoveTo of a unit without speed (state 13)"));
            }
            MoveTo::Start
        } else {
            MoveTo::Arrived
        });
        Ok(())
    }

    /// `set_state(6)` (case 6): the hunt faces its target. Live livestock of its own player is led
    /// to its drop site first when it stands more than 3 tiles from it (`HERD_REACH` squared,
    /// `set_state` 10, not modelled); then live livestock is attacked (`set_state` 12, with an Attack
    /// of the master's ranges: [`Herd`]). Dead livestock out of the task's range plus `GAP` is
    /// walked to again (`set_state` 4, [`HuntAction::set_walk`]). Otherwise the unit takes the hunt's sprite
    /// (`FUN_140ac51d0`: the carrying one for a load of the task's resource of 1 or more, else the
    /// moving one; slot 0x68 takes the standing sprite for none). A target of class 49 (its
    /// owner's change, `FUN_141185360`) is not modelled.
    fn set_face(
        &mut self,
        walk: Walk<'_>,
        strike: &mut Strike<'_>,
        gather: &mut Gatherer<'_>,
        herd: Option<Herd>,
    ) -> Result<(), Unsupported> {
        if gather.target_class == CLASS_49 {
            return Err(Unsupported(
                "a hunt facing a target of class 49 (set_state 6)",
            ));
        }
        if gather.target_class == LIVESTOCK {
            if strike.victim.state <= 2 {
                let herd = herd.ok_or(Unsupported("a hunt facing live livestock unprepared"))?;
                // own livestock more than 3 tiles from its drop site is led there: once sent
                // (`moved_sheep`) the hunt waits for it; nearer than 30 tiles an animal that
                // stands is sent (its slot 0x138) with the herd around it (`FUN_140ac4c60`, the
                // caller's); either way `moved_sheep` and `set_state(10)`
                if let (true, Some((site, gap_squared))) = (herd.own, herd.drop_site) {
                    self.drop_site = site;
                    if HERD_REACH < gap_squared && (self.lead.moved || gap_squared < HERD_FAR) {
                        if !self.lead.moved && herd.idle {
                            gather.leads = Some(site);
                        }
                        self.lead.moved = true;
                        return self.set_lead(walk, strike, &herd);
                    }
                }
                return self.start_attack(walk, strike, &herd);
            }
            let t = walk
                .world
                .get_known(self.target)?
                .ok_or(Unsupported("a hunt's target without a place"))?;
            let me = walk.me.position;
            let ground = walk
                .mover
                .as_deref()
                .map(|m| [m.stats.radius[0], m.stats.radius[1]])
                .ok_or(Unsupported("a hunter without movement state"))?;
            if self.task.range + GAP < footprint_gap(me, ground, t.position, t.radius) {
                let mut walk = walk;
                let player = strike.own.player;
                return self.set_walk(&mut walk, strike.victim, player);
            }
        }
        self.move_to = None;
        self.state = FACE;
        let carrying = self.task.resource != -1
            && *gather.held_kind == self.task.resource
            && 1 <= *gather.held as i32;
        let g = match (carrying, gather.sprites.carrying) {
            (true, Some(g)) => Some(g),
            _ => gather.sprites.moving,
        }
        .or(gather.sprites.standing);
        if let Some(g) = g {
            set_sprite(strike.sprite, g, strike.sprite_rng);
        }
        Ok(())
    }

    /// `set_state(7)` (case 7): a food task keeps a load of food (`FUN_14119d310` keeps the
    /// amount), any other load is dropped; the unit takes the working sprite (slot 0x68; the
    /// standing one for none); the gathering sound is not held.
    fn set_gather(&mut self, strike: &mut Strike<'_>, gather: &mut Gatherer<'_>) {
        self.state = GATHER;
        let kind = self.task.resource;
        let keeps = food(kind) && food(*gather.held_kind);
        if *gather.held_kind != kind {
            *gather.held = if keeps { *gather.held } else { 0.0 };
            *gather.held_kind = kind;
        }
        if let Some(g) = gather.sprites.working.or(gather.sprites.standing) {
            set_sprite(strike.sprite, g, strike.sprite_rng);
        }
    }

    /// State 7 (case 6): the work, `(work rate) × work_val1 × dt`, at most what the target holds,
    /// times the productivity, while the load stays within the capacity: the target loses the
    /// work, its last bit below 0.1 going with a thousandth of its master's first storage amount;
    /// the unit gains, a remainder below 0.1 of the capacity rounding the load up. A full load or
    /// an empty target ends the gathering: true asks the caller for `set_state(8)` (its drop site
    /// search, [`HuntAction::set_drop`]). The game's `+0xc14`
    /// (`FUN_140c884c0`), 0 as the tick oracle holds it, would take the whole target at once.
    fn gather(
        &self,
        dt: f32,
        victim: &mut Victim,
        g: &mut Gatherer<'_>,
    ) -> Result<bool, Unsupported> {
        let work = (g.work_rate * self.task.work_value) * dt;
        let take = if work <= victim.amount {
            work
        } else {
            victim.amount
        };
        let mut gain = g.productivity * take;
        let capacity = g.capacity;
        // `<=` in the game: a NaN goes to the drop-off too
        let room = capacity - *g.held;
        if !matches!(
            gain.partial_cmp(&room),
            Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
        ) {
            return Ok(true);
        }
        victim.amount -= take;
        // the last bit and the storage's share summed first (`0x140ac8e04`)
        if victim.amount < REMAINDER {
            gain += g.target_storage * LAST_BIT + victim.amount;
            victim.amount = 0.0;
        }
        *g.held += gain;
        let over = (*g.held - capacity).abs();
        if *g.held < capacity && over < REMAINDER && over <= victim.amount {
            victim.amount -= over;
            *g.held += over;
        }
        if matches!(g.target_class, 9 | 10) && *g.held_kind == 3 && self.task.resource == 3 {
            return Err(Unsupported(
                "a gold load of class 9 or 10 (its player's attribute 3)",
            ));
        }
        Ok(!(REMAINDER <= over && 0.0 < victim.amount))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// No sprite shown: a test hunt's walk.
    fn shown<'a>(
        sprite: &'a mut Option<crate::sprite::ActiveSprite>,
        rng: &'a mut crate::rng::Well512,
    ) -> Shown<'a> {
        Shown {
            sprite,
            rng,
            graphic: None,
        }
    }
    use crate::movement::{MoveStats, Velocity};

    /// A Hunt gathering `target` with the chopping task.
    pub(crate) fn hunting(target: i32) -> HuntAction {
        HuntAction {
            target,
            state: GATHER,
            point: [0.0; 3],
            target_class: -1,
            task: CHOP,
            attack: None,
            move_to: None,
            walk_range: 0.0,
            drop_site: -1,
            carry_to: -1,
            timer: 0.0,
            move_wait: MoveWait::default(),
            lead: Lead::default(),
        }
    }

    /// `FUN_140ac44b0`: the decay task (`attribute_type4` 0x124) under attribute 292 turns a
    /// decaying target's decay negative once; any other task or a player without it leaves it.
    #[test]
    fn a_decay_task_stops_its_target_s_decay_once() {
        let mut hunt = hunting(7);
        let decaying = || Victim {
            decay: 0.25,
            ..Victim::default()
        };
        let mut v = decaying();
        hunt.decay_check(true, &mut v);
        assert_eq!(v.decay, 0.25);
        hunt.task.kind4 = DECAY_TASK;
        hunt.decay_check(false, &mut v);
        assert_eq!(v.decay, 0.25);
        hunt.decay_check(true, &mut v);
        assert_eq!(v.decay, -0.25);
        hunt.decay_check(true, &mut v);
        assert_eq!(v.decay, -0.25);
        let mut none = Victim::default();
        hunt.decay_check(true, &mut none);
        assert_eq!(none.decay.to_bits(), 0);
    }

    /// A villager walking at 0.8 tiles a second, radius 0.2.
    pub(crate) fn walker() -> Mover {
        Mover {
            forward: [-1.0, 0.0, 0.0],
            right: [0.0, 0.0, 1.0],
            velocity: Some(Velocity {
                velocity: [0.0, 0.0, 0.8],
                acceleration: [0.0, 0.0, 1.0],
            }),
            paths: vec![],
            factor: 1.0,
            stats: MoveStats {
                fly_mode: 0,
                speed: 0.8,
                turn_rate: f32::MAX,
                turn_radius: 0.0,
                turn_radius_speed: f32::MAX,
                radius: [0.2, 0.2, 0.5],
                restriction: 7,
                restriction_mask: 6,
                turn_time: 0.0,
                move_algorithm: 0,
                min_collision: 1.0,
                fast_turn: false,
                walk_speed: 1.0,
                stand_speed: 1.0,
                run_speed: 1.0,
                facets: Some(crate::facing::SpriteFacets {
                    count: 16,
                    flag_1: true,
                }),
                weapon_range: 0.0,
            },
            waiting: 1,
            wait_delays: 0,
            attack_moving: false,
            last_move: 0,
            moved_at: [0.0; 3],
            substitute_count: 0,
            ignore_units: Vec::new(),
            user_waypoints: Vec::new(),
            last_turn: 0.0,
            angle: 0.0,
            move_to_point: None,
            speed_stopped: false,
            pauses: Vec::new(),
            in_the_way: Vec::new(),
            sprite_facets: None,
        }
    }

    /// MarvinBlue/426048731's tree 9365 (Gaia, class 15, 100 wood) as 9181 meets it in update 748.
    fn tree() -> Quarry {
        Quarry {
            id: 9365,
            position: [24.5, 97.5, 0.0],
            class: 15,
            kind: 10,
            state: 2,
            player: 0,
            amount: 100.0,
            allied: false,
            top_action: None,
            drop_site: false,
        }
    }

    fn engagement() -> Engagement {
        Engagement {
            gap_squared: 3.726 * 3.726,
            gap: 3.726,
            position: [20.774, 97.5, 0.0],
            target_position: [24.5, 97.5, 0.0],
            speed: 0.8,
            range: 0.0,
            min_range: 0.0,
            can_move: true,
            stance: 0,
            world_time: 0,
        }
    }

    const ARMS: Arms = Arms {
        move_sprite: None,
        sprite: None,
        second_sprite: None,
        idle_sprite: None,
        idle_attack_sprite: None,
        missile: -1,
        fire_frame: 0,
        interrupt_frame: -1,
    };

    const CHOP: HuntTask = HuntTask {
        id: 0,
        range: 0.0,
        resource: 15,
        sprites: [-1, 1239, 1961, 1960],
        work_value: 1.0,
        productivity: -1,
        kind4: -1,
    };

    #[test]
    fn a_villager_sent_to_a_tree_attacks_it_first_and_walks_there() {
        let mut mover = walker();
        let mut held = (-1, 0.0);
        let (hunt, _) = HuntAction::start(
            CHOP,
            &tree(),
            2,
            false,
            false,
            &mut held,
            &engagement(),
            ARMS,
            &mut mover,
        )
        .unwrap();
        assert_eq!(hunt.state, ATTACK);
        let attack = hunt.attack.unwrap();
        assert_eq!((attack.state, attack.move_to), (4, Some(MoveTo::Start)));
        // the first MoveTo's teardown stopped the unit
        let v = mover.velocity.unwrap();
        assert_eq!(
            (v.velocity[2], v.acceleration[2], mover.waiting),
            (0.0, 0.0, 0)
        );
        // nothing carried, a human player and settings that keep it: the held kind stays
        assert_eq!(held, (-1, 0.0));
        // the constructor's lead: the animal's start at (-1, -1)
        assert_eq!(hunt.lead, Lead::NEW);
        assert_eq!(hunt.lead.start, [-1.0, -1.0]);
    }

    /// Slot 0x48 on another tree: the hunt takes it and its Attack starts over; one within 0.01
    /// starts arrived, which the caller shows (records/425906646's 31110 in update 7967). Its own
    /// target it does not take.
    #[test]
    fn a_hunt_retargeted_to_a_tree_beside_it_starts_its_attack_arrived() {
        let mut mover = walker();
        let mut held = (-1, 0.0);
        let (mut hunt, _) = HuntAction::start(
            CHOP,
            &tree(),
            2,
            false,
            false,
            &mut held,
            &engagement(),
            ARMS,
            &mut mover,
        )
        .unwrap();
        let retarget = |hunt: &mut HuntAction, q: &Quarry, e: &Engagement, m: &mut Mover| {
            hunt.retarget(q, 2, 0.0, false, e, ARMS, m).unwrap()
        };
        assert_eq!(
            retarget(&mut hunt, &tree(), &engagement(), &mut mover),
            None
        );
        let beside = Quarry {
            id: 9366,
            position: [20.78, 97.5, 0.0],
            ..tree()
        };
        let close = Engagement {
            gap_squared: 0.005 * 0.005,
            gap: 0.005,
            target_position: beside.position,
            ..engagement()
        };
        assert_eq!(retarget(&mut hunt, &beside, &close, &mut mover), Some(true));
        let attack = hunt.attack.unwrap();
        assert_eq!(
            (hunt.target, hunt.state, attack.state, attack.move_to),
            (9366, ATTACK, 4, Some(MoveTo::Arrived))
        );
    }

    #[test]
    fn a_hunter_keeps_another_food_it_carries_and_drops_the_rest_by_the_settings() {
        let start = |held: (i16, f32), carry_rule: bool| {
            let mut held = held;
            let _ = HuntAction::start(
                CHOP,
                &tree(),
                2,
                false,
                carry_rule,
                &mut held,
                &engagement(),
                ARMS,
                &mut walker(),
            )
            .unwrap();
            held
        };
        assert_eq!(start((0, 7.0), false), (15, 7.0));
        assert_eq!(start((2, 7.0), false), (2, 7.0));
        assert_eq!(start((2, 7.0), true), (15, 0.0));
    }

    fn victim(amount: f32) -> Victim {
        Victim {
            id: 9357,
            amount,
            state: 3,
            kind: 10,
            class: 15,
            storage: 100.0,
            ..Victim::default()
        }
    }

    #[test]
    fn a_villager_gathers_work_rate_times_work_value_times_dt_until_its_load_is_full() {
        let hunt = HuntAction {
            target: 9357,
            state: GATHER,
            point: [24.5, 94.5, 0.0],
            target_class: 15,
            task: HuntTask {
                work_value: 0.39,
                resource: 1,
                ..CHOP
            },
            attack: None,
            move_to: None,
            walk_range: 0.0,
            drop_site: -1,
            carry_to: -1,
            timer: 0.0,
            move_wait: MoveWait::default(),
            lead: Lead::default(),
        };
        let (mut kind, mut held) = (1, 0.0f32);
        let gather = |held: &mut f32, kind: &mut i16, tree: &mut Victim| {
            let mut g = Gatherer {
                held_kind: kind,
                held,
                work_rate: 1.0,
                capacity: 10.0,
                productivity: 1.0,
                target_class: 15,
                target_storage: 100.0,
                stops_decay: false,
                ai_class: None,
                sprites: HuntSprites::default(),
                tree_searched: false,
                predator_hunts_end: false,
                searches: false,
                leads: None,
                attack_failed: false,
                let_go: None,
            };
            hunt.gather(0.02, tree, &mut g)
        };
        let mut tree = victim(100.0);
        assert_eq!(gather(&mut held, &mut kind, &mut tree), Ok(false));
        let w = (1.0f32 * 0.39) * 0.02;
        assert_eq!((held, tree.amount), (w, 100.0 - w));
        // a load 0.1 short of the capacity rounds up, and the full load asks for set_state(8)
        held = 9.95 - w;
        let mut tree = victim(50.0);
        assert_eq!(gather(&mut held, &mut kind, &mut tree), Ok(true));
        assert_eq!(held, 10.0);
        // a tree's last bit goes with a thousandth of its master's storage, the empty target
        // asking for set_state(8)
        held = 0.0;
        let mut tree = victim(0.05);
        assert_eq!(gather(&mut held, &mut kind, &mut tree), Ok(true));
        assert_eq!((held, tree.amount), (0.05 + 100.0 * 0.001, 0.0));
    }

    /// Case 6 without a target: a load is carried off (`set_state(8)`), none or a NaN load searches
    /// (`set_state(3)`, `COMISS` with `JBE`).
    #[test]
    fn a_hunt_without_a_target_carries_its_load_off_or_searches() {
        let mut h = HuntAction {
            target: -1,
            state: GATHER,
            point: [0.0; 3],
            target_class: 15,
            task: CHOP,
            attack: None,
            move_to: None,
            walk_range: 0.0,
            drop_site: -1,
            carry_to: -1,
            timer: 0.0,
            move_wait: MoveWait::default(),
            lead: Lead::default(),
        };
        assert!(h.ends_without_target(0.5));
        assert_eq!(h.state, GATHER);
        for held in [0.0, -1.0, f32::NAN] {
            h.state = GATHER;
            assert!(!h.ends_without_target(held));
            assert_eq!(h.state, SEARCH);
        }
    }

    /// A live allied target fails the hunt at once (`set_state` 13, its timer cleared), as house
    /// 9158 did MarvinBlue/426048731's 9181 after its drop-off in update 1897.
    #[test]
    fn a_hunt_on_a_live_ally_fails() {
        let ally = Quarry {
            allied: true,
            ..tree()
        };
        let mut held = (-1, 0.0);
        let (hunt, _) = HuntAction::start(
            CHOP,
            &ally,
            2,
            false,
            false,
            &mut held,
            &engagement(),
            ARMS,
            &mut walker(),
        )
        .expect("a hunt");
        assert_eq!(
            (hunt.state, hunt.timer, hunt.target),
            (FAILED, 0.0, ally.id)
        );
    }

    /// `set_state(4)` on a dead target, or on the hunter's own livestock, walks to it: a quiet
    /// `MoveTo` within the task's range, or 1.5 to the livestock (MarvinBlue/426048731's 9171 to its
    /// own sheep 9520 in update 1116).
    #[test]
    fn a_hunt_walks_to_a_dead_target_or_its_own_livestock() {
        let dead = Quarry { state: 3, ..tree() };
        let sheep = Quarry {
            class: LIVESTOCK,
            player: 2,
            ..tree()
        };
        for (q, range) in [(dead, CHOP.range), (sheep, 1.5)] {
            let mut held = (-1, 0.0);
            let (hunt, _) = HuntAction::start(
                CHOP,
                &q,
                2,
                false,
                false,
                &mut held,
                &engagement(),
                ARMS,
                &mut walker(),
            )
            .unwrap();
            assert_eq!(
                (hunt.state, hunt.move_to, hunt.walk_range, hunt.attack),
                (WALK, Some(MoveTo::Start), range, None)
            );
        }
    }

    /// 425906646's shepherd 31110 in update 3543: the carcass it walks to went past state 6, so
    /// its walk ends and a new one to the hunt's point starts (state 11), the unit stopped; a
    /// lumberjack (its unit AI's class 15) would search for another tree first.
    #[test]
    fn a_hunt_whose_target_went_walks_to_its_point() {
        let mut hunt = HuntAction {
            target: 28813,
            state: WALK,
            point: [177.272_75, 62.444_31, 0.0],
            target_class: LIVESTOCK,
            task: CHOP,
            attack: None,
            move_to: Some(MoveTo::Moving),
            walk_range: CHOP.range,
            drop_site: -1,
            carry_to: -1,
            timer: 0.0,
            move_wait: MoveWait::default(),
            lead: Lead::default(),
        };
        let mut mover = walker();
        let mut world = crate::spatial::Spatial::default();
        let (mut kind, mut held) = (-1, 0.0);
        let mut walk = Walk {
            mover: Some(&mut mover),
            me: crate::movement::Me {
                id: 31110,
                position: [171.888_89, 59.053_39, 0.0],
                group: -1,
                action_state: WALK,
            },
            dt: 0.028,
            world: &mut world,
            plan: None,
        };
        let mut gather = Gatherer {
            held_kind: &mut kind,
            held: &mut held,
            work_rate: 1.0,
            capacity: 10.0,
            productivity: 1.0,
            target_class: LIVESTOCK,
            target_storage: 0.0,
            stops_decay: false,
            ai_class: Some(15),
            sprites: HuntSprites::default(),
            tree_searched: false,
            predator_hunts_end: false,
            searches: false,
            leads: None,
            attack_failed: false,
            let_go: None,
        };
        assert!(hunt.clone().lost_on_the_way(&mut walk, &gather).is_err());
        // the caller's search for the next tree found none: the hunt walks to its point
        gather.tree_searched = true;
        hunt.clone().lost_on_the_way(&mut walk, &gather).unwrap();
        gather.tree_searched = false;
        gather.ai_class = Some(-1);
        hunt.lost_on_the_way(&mut walk, &gather).unwrap();
        assert_eq!(
            (hunt.state, hunt.move_to, hunt.timer, hunt.point[0]),
            (WALK_SEARCH, Some(MoveTo::Start), 0.0, 177.272_75)
        );
        assert_eq!(mover.velocity.unwrap().velocity[2], 0.0);
    }

    /// A hunt's walk to its point (states 11 and 15, `0x140ac7a00` cases 0xa and 0xe): an ended
    /// `MoveTo` (code 1) stops the unit, after which state 11 searches (`set_state` 3) and state 15
    /// is done (`set_state` 2); a failed one (code 3) does the same for a human player, a computer
    /// player's would end the hunt (`set_state` 13). A human player's villager whose motion step
    /// gave up waits first (425906646's p4 villagers 27238, 31018, 31034 and 31042 in update 5724).
    #[test]
    fn a_walk_to_the_point_ends_in_a_search_or_done() {
        let hunt = |state, move_to| HuntAction {
            target: -1,
            state,
            point: [177.272_75, 62.444_31, 0.0],
            target_class: LIVESTOCK,
            task: CHOP,
            attack: None,
            move_to: Some(move_to),
            walk_range: CHOP.range,
            drop_site: -1,
            carry_to: -1,
            timer: 0.0,
            move_wait: MoveWait::default(),
            lead: Lead::default(),
        };
        let walker_at = PointWalker {
            computer: false,
            villager: true,
            stance: 1,
        };
        let run = |h: &mut HuntAction, who: PointWalker| {
            let mut mover = walker();
            let mut world = crate::spatial::Spatial::default();
            let (mut sprite, mut rng) = (None, crate::rng::Well512::default());
            let walk = Walk {
                mover: Some(&mut mover),
                me: crate::movement::Me {
                    id: 31110,
                    position: [171.888_89, 59.053_39, 0.0],
                    group: -1,
                    action_state: h.state,
                },
                dt: 0.5,
                world: &mut world,
                plan: None,
            };
            let got = h.update_point_walk(walk, who, shown(&mut sprite, &mut rng));
            (got, mover.velocity.unwrap().velocity[2])
        };
        let done = PointWalk::Done {
            target: -1,
            second: -1,
        };
        for (state, ends) in [(WALK_SEARCH, PointWalk::Search), (WALK_POINT, done)] {
            for code in [MoveTo::Arrived, MoveTo::Failed] {
                let mut h = hunt(state, code);
                let (got, speed) = run(&mut h, walker_at);
                assert_eq!((got.unwrap(), h.move_to, speed), (ends, None, 0.0));
                let after = if ends == PointWalk::Search {
                    SEARCH
                } else {
                    IDLE
                };
                assert_eq!(h.state, after);
            }
            // a computer player's failed walk ends the hunt (set_state 13)
            let computer = PointWalker {
                computer: true,
                ..walker_at
            };
            assert!(run(&mut hunt(state, MoveTo::Failed), computer).0.is_err());
        }
        // set_state(2) lets the target and the second target go, in that order
        let mut h = hunt(SEARCH, MoveTo::Arrived);
        (h.target, h.carry_to, h.timer) = (28813, 9156, -10.0);
        assert_eq!(h.set_done(), [28813, 9156]);
        assert_eq!(
            (h.state, h.target, h.carry_to, h.point, h.timer),
            (IDLE, -1, -1, [-1.0; 3], 0.0)
        );
        // the wait: a second for a stance below 2, then the MoveTo asks again
        let mut h = hunt(WALK_SEARCH, MoveTo::Waiting);
        h.move_wait = MoveWait {
            state: 1,
            timer: 0.0,
            delay: 1.0,
        };
        assert_eq!(run(&mut h, walker_at).0.unwrap(), PointWalk::Walking);
        assert_eq!((h.move_to, h.move_wait.timer), (Some(MoveTo::Waiting), 0.5));
        assert_eq!(run(&mut h, walker_at).0.unwrap(), PointWalk::Walking);
        assert_eq!((h.move_to, h.move_wait.timer), (Some(MoveTo::Start), 0.0));
        // the MoveTo's answer to the motion step giving up: a human's villager waits once
        let mut h = hunt(WALK_POINT, MoveTo::Moving);
        h.give_up_walk(walker_at);
        assert_eq!(
            (h.move_to, h.move_wait.state, h.move_wait.delay),
            (Some(MoveTo::Waiting), 1, 1.0)
        );
        h.give_up_walk(walker_at);
        assert_eq!(h.move_to, Some(MoveTo::Failed));
        let mut h = hunt(WALK_POINT, MoveTo::Moving);
        h.give_up_walk(PointWalker {
            villager: false,
            ..walker_at
        });
        assert_eq!(h.move_to, Some(MoveTo::Failed));
    }

    /// MarvinBlue/426048731's lumberjack 9181 in update 1897: its `MoveTo` to the TC 9156 ended
    /// (state 1); the TC takes wood within a tile, so the load goes to player 2 as wood (1) and
    /// the unit carries nothing after; the hunt has no target to go back to.
    #[test]
    fn a_drop_off_gives_the_load_to_the_player() {
        fn walk<'a>(mover: &'a mut Mover, world: &'a mut crate::spatial::Spatial) -> Walk<'a> {
            Walk {
                mover: Some(mover),
                me: crate::movement::Me {
                    id: 9181,
                    position: [26.820_372, 94.718_056, 0.0],
                    group: -1,
                    action_state: DROP,
                },
                dt: 0.028,
                world,
                plan: None,
            }
        }
        let mut hunt = HuntAction {
            target: -1,
            state: DROP,
            point: [-1.0; 3],
            target_class: 3,
            task: CHOP,
            attack: None,
            move_to: Some(MoveTo::Arrived),
            walk_range: CHOP.range,
            drop_site: -1,
            carry_to: 9156,
            timer: 0.0,
            move_wait: MoveWait::default(),
            lead: Lead::default(),
        };
        let mut mover = walker();
        let mut world = crate::spatial::Spatial::default();
        let (mut sprite, mut rng) = (None, crate::rng::Well512::default());
        let (mut kind, mut held) = (1, 7.057_053_6);
        let site = DropOff {
            takes: true,
            gap_squared: 0.5,
        };
        let who = PointWalker {
            computer: false,
            villager: true,
            stance: 0,
        };
        let mut anim = Some(1.0);
        let got = hunt
            .update_drop(
                walk(&mut mover, &mut world),
                &mut kind,
                &mut held,
                230,
                Some(site),
                shown(&mut sprite, &mut rng),
                who,
                &mut anim,
            )
            .unwrap();
        assert_eq!(got, Dropped::Load(1, 7.057_053_6));
        assert_eq!((kind, held, hunt.move_to), (-1, 0.0, None));
        // carried food of a carcass (15) goes as food; a site that does not take it stops
        let mut kind = 15;
        let mut held = 2.0;
        hunt.move_to = Some(MoveTo::Arrived);
        let got = hunt
            .update_drop(
                walk(&mut mover, &mut world),
                &mut kind,
                &mut held,
                230,
                Some(site),
                shown(&mut sprite, &mut rng),
                who,
                &mut anim,
            )
            .unwrap();
        assert_eq!(got, Dropped::Load(0, 2.0));
        hunt.move_to = Some(MoveTo::Arrived);
        let refused = DropOff {
            takes: false,
            ..site
        };
        assert!(
            hunt.update_drop(
                walk(&mut mover, &mut world),
                &mut kind,
                &mut held,
                230,
                Some(refused),
                shown(&mut sprite, &mut rng),
                who,
                &mut anim,
            )
            .is_err()
        );
        // a site out of a tile's reach: the hunt waits 3 seconds, the unit's animation stopped;
        // at the wait's end it tries again
        hunt.move_to = Some(MoveTo::Arrived);
        let far = DropOff {
            gap_squared: 1.5,
            ..site
        };
        let step = |hunt: &mut HuntAction, mover: &mut Mover, anim: &mut Option<f32>| {
            let mut world = crate::spatial::Spatial::default();
            let (mut sprite, mut rng) = (None, crate::rng::Well512::default());
            hunt.update_drop(
                walk(mover, &mut world),
                &mut 15,
                &mut 2.0,
                230,
                Some(far),
                shown(&mut sprite, &mut rng),
                who,
                anim,
            )
            .unwrap()
        };
        assert_eq!(step(&mut hunt, &mut mover, &mut anim), Dropped::Going);
        assert_eq!((hunt.timer, anim), (DROP_WAIT, Some(0.0)));
        hunt.timer = -0.01;
        assert_eq!(step(&mut hunt, &mut mover, &mut anim), Dropped::Again);
        assert_eq!(hunt.timer, 0.0);
    }
}
