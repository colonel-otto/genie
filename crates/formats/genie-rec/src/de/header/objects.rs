//! A view of the generated object layouts (`generated/world.rs`): the parts the object classes
//! share. Each class's struct holds its base class's as its first field, as the game's loads call
//! the base load first; the factories hold one optional struct per object type.

use super::generated::world::{
    Master, Object, ObjectList, OwnRgeMasterActionObject, OwnRgeMasterStaticObject,
    OwnTribeMasterCombatObject, Player, PlayerObjects, RgeActionObject, RgeMasterActionObject,
    RgeMasterMovingObject, RgeMasterStaticObject, RgeMovingObject, RgeObject, RgeStaticObject,
    TribeCombatObject, TribeMasterCombatObject, UnitAiModule, WorldPlayer,
};

impl Player {
    /// The player load of its class (`WorldPlayer.cpp`); None for a type no creator handles.
    pub fn world_player(&self) -> Option<&WorldPlayer> {
        None.or(self
            .scenario_editor_phantom_player
            .as_ref()
            .map(|p| &p.world_player_scenario_editor_phantom))
            .or(self
                .human_or_coop_player
                .as_ref()
                .map(|p| &p.world_player_human_or_coop))
            .or(self
                .gaia_player
                .as_ref()
                .map(|p| &p.world_player_gaia.world_player))
            .or(self
                .computer_player
                .as_ref()
                .map(|p| &p.world_player_computer))
    }

    /// The masters, maps and objects (WorldPlayer vtable slot 29).
    pub fn objects(&self) -> Option<&PlayerObjects> {
        None.or(self
            .scenario_editor_phantom_player
            .as_ref()
            .map(|p| &p.player_objects))
            .or(self
                .human_or_coop_player
                .as_ref()
                .map(|p| &p.player_objects))
            .or(self.gaia_player.as_ref().map(|p| &p.player_objects))
            .or(self.computer_player.as_ref().map(|p| &p.player_objects))
    }
}

impl PlayerObjects {
    /// The player's master of type `id`; None if the save holds none (a master the player never
    /// enabled).
    pub fn master(&self, id: i16) -> Option<&Master> {
        self.masters
            .iter()
            .flatten()
            .map(|m| &m.master)
            .find(|m| m.static_part().is_some_and(|s| s.id() == id))
    }
}

impl Master {
    /// The static master every class starts with.
    pub fn static_part(&self) -> Option<&RgeMasterStaticObject> {
        if let Some(t) = &self.tribe_master_tree_object {
            return Some(t);
        }
        if let Some(c) = &self.tribe_master_combat_object {
            return Some(tribe_combat_master(c));
        }
        if let Some(b) = &self.tribe_master_building_object {
            return Some(tribe_combat_master(&b.tribe_master_combat_object));
        }
        let r = self.rge_master.as_ref()?;
        None.or(r.rge_master_static_object.as_ref())
            .or(r
                .rge_master_animated_object
                .as_ref()
                .map(|a| &a.rge_master_static_object))
            .or(r
                .rge_master_doppleganger_object
                .as_ref()
                .map(|d| &d.rge_master_static_object))
            .or(r.rge_master_moving_object.as_ref().map(moving_master))
            .or(r.rge_master_action_object.as_ref().map(action_master))
            .or(r
                .rge_master_combat_object
                .as_ref()
                .map(|c| action_master(&c.rge_master_action_object)))
            .or(r
                .rge_master_missile_object
                .as_ref()
                .map(|m| action_master(&m.rge_master_combat_object.rge_master_action_object)))
    }
}

impl RgeMasterStaticObject {
    /// The master's type id: the save writes it first (a `readTemp` local).
    pub fn id(&self) -> i16 {
        self.read_temp
    }
}

fn moving_master(m: &RgeMasterMovingObject) -> &RgeMasterStaticObject {
    &m.rge_master_animated_object.rge_master_static_object
}

fn action_master(a: &RgeMasterActionObject) -> &RgeMasterStaticObject {
    moving_master(&a.rge_master_moving_object)
}

fn tribe_combat_master(c: &TribeMasterCombatObject) -> &RgeMasterStaticObject {
    action_master(&c.rge_master_combat_object.rge_master_action_object)
}

impl RgeStaticObject {
    /// The static part of the object's own master (`has_own_master_object`), which the object
    /// uses instead of its player's.
    pub fn own_master(&self) -> Option<&OwnRgeMasterStaticObject> {
        None.or(self.rge_master_static_object.as_ref())
            .or(self
                .rge_master_animated_object
                .as_ref()
                .map(|a| &a.own_rge_master_static_object))
            .or(self.rge_master_moving_object.as_ref().map(|m| {
                &m.own_rge_master_animated_object
                    .own_rge_master_static_object
            }))
            .or(self
                .rge_master_action_object
                .as_ref()
                .map(own_action_master))
            .or(self.rge_master_missile_object.as_ref().map(|m| {
                own_action_master(&m.own_rge_master_combat_object.own_rge_master_action_object)
            }))
            .or(self
                .tribe_master_combat_object
                .as_ref()
                .map(own_tribe_combat_master))
            .or(self
                .tribe_master_building_object
                .as_ref()
                .map(|b| own_tribe_combat_master(&b.own_tribe_master_combat_object)))
    }
}

fn own_action_master(a: &OwnRgeMasterActionObject) -> &OwnRgeMasterStaticObject {
    &a.own_rge_master_moving_object
        .own_rge_master_animated_object
        .own_rge_master_static_object
}

fn own_tribe_combat_master(c: &OwnTribeMasterCombatObject) -> &OwnRgeMasterStaticObject {
    own_action_master(&c.own_rge_master_combat_object.own_rge_master_action_object)
}

impl ObjectList {
    /// The objects in list order, without the entry of type 0 that ends the list.
    pub fn objects(&self) -> impl Iterator<Item = &Object> {
        self.object_type_list
            .iter()
            .filter_map(|e| e.object.as_ref())
    }
}

impl Object {
    /// The static object every class starts with.
    pub fn static_part(&self) -> Option<&RgeStaticObject> {
        if let Some(c) = self.combat() {
            return Some(combat_static(c));
        }
        self.rge_object.as_ref().and_then(RgeObject::static_part)
    }

    /// The tribe combat object of a combat object (type 70) or a building (type 80).
    pub fn combat(&self) -> Option<&TribeCombatObject> {
        self.tribe_combat_object.as_ref().or_else(|| {
            self.tribe_building_object
                .as_ref()
                .map(|b| &b.tribe_combat_object)
        })
    }
}

impl RgeObject {
    /// The static object of the base factory's types (10 to 60).
    pub fn static_part(&self) -> Option<&RgeStaticObject> {
        self.rge_static_object
            .as_ref()
            .or_else(|| {
                self.rge_animated_object
                    .as_ref()
                    .map(|a| &a.rge_static_object)
            })
            .or_else(|| {
                self.rge_doppleganger_object
                    .as_ref()
                    .map(|d| &d.rge_animated_object.rge_static_object)
            })
            .or_else(|| self.rge_moving_object.as_ref().map(moving))
            .or_else(|| self.rge_action_object.as_ref().map(action))
            .or_else(|| {
                self.rge_combat_object
                    .as_ref()
                    .map(|c| action(&c.rge_action_object))
            })
            .or_else(|| {
                self.rge_missile_object
                    .as_ref()
                    .map(|m| action(&m.rge_combat_object.rge_action_object))
            })
    }
}

fn moving(m: &RgeMovingObject) -> &RgeStaticObject {
    &m.rge_animated_object.rge_static_object
}

fn action(a: &RgeActionObject) -> &RgeStaticObject {
    moving(&a.rge_moving_object)
}

fn combat_static(c: &TribeCombatObject) -> &RgeStaticObject {
    action(&c.rge_combat_object.rge_action_object)
}

impl TribeCombatObject {
    /// The unit AI, whatever class its type made: the base module, or the elephant, tower or
    /// building module, which start with it.
    pub fn unit_ai(&self) -> Option<&UnitAiModule> {
        self.unit_ai_module
            .as_ref()
            .or(self.unit_ai_module_2.as_ref())
            .or_else(|| {
                self.tribe_elephant_unit_ai_module
                    .as_ref()
                    .map(|m| &m.unit_ai_module)
            })
            .or_else(|| {
                self.tribe_tower_unit_ai_module
                    .as_ref()
                    .map(|m| &m.unit_ai_module)
            })
            .or_else(|| {
                self.tribe_building_unit_ai_module
                    .as_ref()
                    .map(|m| &m.unit_ai_module)
            })
    }
}
