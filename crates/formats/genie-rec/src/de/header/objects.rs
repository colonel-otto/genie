//! A view of the generated object layouts (`generated/world.rs`): the parts the object classes
//! share. Each class's struct holds its base class's as its first field, as the game's loads call
//! the base load first; the factories hold one optional struct per object type.

use super::generated::world::{
    Object, ObjectList, Player, PlayerObjects, RgeActionObject, RgeMovingObject, RgeObject,
    RgeStaticObject, TribeCombatObject, UnitAiModule, WorldPlayer,
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
