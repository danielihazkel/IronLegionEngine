//! `BattleFrame`: an owned copy of what the presentation layer reads, taken
//! after a step (T3-032, TDD §4.2, SAD §8). The app's sim thread captures
//! one per published tick and hands it to the main thread, which renders,
//! picks, orders and plays sound from it while the next tick runs; the
//! methods carry `BattleView`'s names so the readers did not change shape.
//!
//! Capturing reads the world only: it never touches the hash. The map, the
//! nav grid and the escape fields are fixed once the sides exist (they
//! change with gates in Phase 5), so a session shares them through
//! [`FrameStatics`] instead of copying them every tick.

use std::sync::Arc;

use il_core::{RegimentId, S, Scalar, SoldierId, Tick};
use il_data::{FormationTemplate, Handle, Registries};

use crate::ai::ArmyPlan;
use crate::components::{FormationState, Path, UnitGroup};
use crate::flow::FlowField;
use crate::interface::BattleSetup;
use crate::map::LoadedMap;
use crate::nav::NavGrid;
use crate::resources::{BattlePhase, SideState};
use crate::spatial::SpatialGrid;
use crate::view::{AbilityRow, BattleView, ProjectileRow, RegimentRow, SoldierRow, StatusRow};
use crate::visibility::Seen;

/// What a session's frames share: built once from the world after spawn.
#[derive(Clone, Debug)]
pub struct FrameStatics {
    pub map: Arc<LoadedMap>,
    pub nav: Arc<NavGrid>,
    /// One escape field per side (SIM-FLOW-001).
    pub flow: Arc<Vec<FlowField>>,
    pub setup: Option<Arc<BattleSetup>>,
}

impl FrameStatics {
    pub fn of(world: &crate::BattleWorld) -> Self {
        let view = world.view();
        let sides = view.sides().len();
        Self {
            map: Arc::new(view.map().clone()),
            nav: Arc::new(view.nav_grid().clone()),
            flow: Arc::new(
                (0..sides)
                    .map_while(|s| view.flow_field(s as u8).cloned())
                    .collect(),
            ),
            setup: world.setup().cloned().map(Arc::new),
        }
    }
}

/// The heavy parts only the debug overlays read; captured only while one
/// of them is on (T3-032).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameDetail {
    /// Formation slots (the slots and paths overlays).
    pub formation: bool,
    /// Regiment paths.
    pub paths: bool,
    /// The soldier grid's cells.
    pub spatial: bool,
    /// The engine's army plans.
    pub ai: bool,
}

impl FrameDetail {
    /// Everything (tests, tools).
    pub const ALL: Self = Self {
        formation: true,
        paths: true,
        spatial: true,
        ai: true,
    };
}

/// One regiment's lists, aligned with `BattleFrame::regiments`.
#[derive(Clone, Debug, Default)]
pub struct RegimentExtras {
    units: Vec<UnitGroup>,
    formations: Vec<Handle<FormationTemplate>>,
    living_by_group: Vec<u16>,
    abilities: Vec<AbilityRow>,
    statuses: Vec<StatusRow>,
    ammo: u16,
    los_radius: S,
    formation: Option<FormationState>,
    path: Option<Path>,
}

/// An owned, read-only copy of the battle after a tick.
#[derive(Clone, Debug)]
pub struct BattleFrame {
    tick: Tick,
    phase: BattlePhase,
    regs: Arc<Registries>,
    statics: Arc<FrameStatics>,
    sides: Vec<SideState>,
    /// Ascending id.
    regiments: Vec<RegimentRow>,
    extras: Vec<RegimentExtras>,
    /// Table order.
    soldiers: Vec<SoldierRow>,
    /// Each side's general while one exists.
    generals: Vec<Option<SoldierRow>>,
    projectiles: Vec<ProjectileRow>,
    /// `side * regiments + index`.
    visible: Vec<bool>,
    seen: Vec<Option<Seen>>,
    ai_plans: Vec<Option<ArmyPlan>>,
    spatial: Option<SpatialGrid<SoldierId>>,
}

impl BattleFrame {
    /// An empty frame over the session's statics; fill it with
    /// [`capture_into`](Self::capture_into).
    pub fn new(regs: Arc<Registries>, statics: Arc<FrameStatics>) -> Self {
        Self {
            tick: Tick::ZERO,
            phase: BattlePhase::Deployment,
            regs,
            statics,
            sides: Vec::new(),
            regiments: Vec::new(),
            extras: Vec::new(),
            soldiers: Vec::new(),
            generals: Vec::new(),
            projectiles: Vec::new(),
            visible: Vec::new(),
            seen: Vec::new(),
            ai_plans: Vec::new(),
            spatial: None,
        }
    }

    /// A frame of the world as it stands (tests and tools).
    pub fn capture(world: &crate::BattleWorld, detail: FrameDetail) -> Self {
        let statics = Arc::new(FrameStatics::of(world));
        let mut frame = Self::new(world.registries().clone(), statics.clone());
        frame.capture_into(&world.view(), &statics, detail);
        frame
    }

    /// Refills the frame from `view`, reusing its buffers.
    pub fn capture_into(
        &mut self,
        view: &BattleView,
        statics: &Arc<FrameStatics>,
        detail: FrameDetail,
    ) {
        self.tick = view.tick();
        self.phase = view.phase();
        if !Arc::ptr_eq(&self.regs, view.regs()) {
            self.regs = view.regs().clone();
        }
        if !Arc::ptr_eq(&self.statics, statics) {
            self.statics = statics.clone();
        }
        self.sides.clear();
        self.sides.extend_from_slice(view.sides());
        self.regiments.clear();
        self.regiments.extend(view.regiments());
        self.soldiers.clear();
        self.soldiers.extend(view.soldiers_unordered());
        self.projectiles.clear();
        self.projectiles.extend(view.projectiles());
        self.generals.clear();
        self.generals.extend(
            self.sides
                .iter()
                .map(|s| s.general.and_then(|id| view.soldier(id))),
        );

        self.extras
            .resize_with(self.regiments.len(), Default::default);
        for (r, x) in self.regiments.iter().zip(self.extras.iter_mut()) {
            x.units.clear();
            x.units.extend_from_slice(view.regiment_units(r.id));
            x.formations = view.regiment_formations(r.id);
            x.living_by_group = view.regiment_living_by_group(r.id);
            x.abilities = view.abilities(r.id);
            x.statuses = view.statuses(r.id);
            x.ammo = view.ammo(r.id);
            x.los_radius = view.los_radius(r.id);
            match (detail.formation, view.formation_state(r.id)) {
                (true, Some(s)) => match &mut x.formation {
                    Some(mine) => mine.clone_from(s),
                    None => x.formation = Some(s.clone()),
                },
                _ => x.formation = None,
            }
            match (detail.paths, view.path(r.id)) {
                (true, Some(p)) => match &mut x.path {
                    Some(mine) => mine.clone_from(p),
                    None => x.path = Some(p.clone()),
                },
                _ => x.path = None,
            }
        }

        let sides = self.sides.len();
        self.visible.clear();
        self.seen.clear();
        for side in 0..sides as u8 {
            for r in &self.regiments {
                self.visible.push(view.visible(side, r.id));
                self.seen.push(view.seen(side, r.id));
            }
        }
        self.ai_plans.clear();
        if detail.ai {
            self.ai_plans
                .extend((0..sides as u8).map(|s| view.ai_plan(s).cloned()));
        }
        match (detail.spatial, &mut self.spatial) {
            (true, Some(grid)) => grid.clone_from(view.spatial_grid()),
            (true, None) => self.spatial = Some(view.spatial_grid().clone()),
            (false, _) => self.spatial = None,
        }
    }

    /// Completed ticks.
    pub fn tick(&self) -> Tick {
        self.tick
    }

    pub fn phase(&self) -> BattlePhase {
        self.phase
    }

    pub fn regs(&self) -> &Arc<Registries> {
        &self.regs
    }

    pub fn statics(&self) -> &Arc<FrameStatics> {
        &self.statics
    }

    /// The battle terrain.
    pub fn map(&self) -> &LoadedMap {
        &self.statics.map
    }

    pub fn nav_grid(&self) -> &NavGrid {
        &self.statics.nav
    }

    /// The side's escape flow field (SIM-FLOW-001).
    pub fn flow_field(&self, side: u8) -> Option<&FlowField> {
        self.statics.flow.get(usize::from(side))
    }

    /// The setup the world was built from (`None` for a bare world).
    pub fn setup(&self) -> Option<&BattleSetup> {
        self.statics.setup.as_deref()
    }

    pub fn sides(&self) -> &[SideState] {
        &self.sides
    }

    pub fn soldier_count(&self) -> usize {
        self.soldiers.len()
    }

    /// Every soldier in table order (no order contract).
    pub fn soldiers_unordered(&self) -> impl Iterator<Item = SoldierRow> + '_ {
        self.soldiers.iter().copied()
    }

    /// The side's general (the aura overlay).
    pub fn general(&self, side: u8) -> Option<SoldierRow> {
        self.generals.get(usize::from(side)).copied().flatten()
    }

    /// Every regiment in ascending `RegimentId` order.
    pub fn regiments(&self) -> impl Iterator<Item = RegimentRow> + '_ {
        self.regiments.iter().copied()
    }

    fn index(&self, id: RegimentId) -> Option<usize> {
        self.regiments.binary_search_by_key(&id, |r| r.id).ok()
    }

    pub fn regiment(&self, id: RegimentId) -> Option<RegimentRow> {
        self.index(id).map(|i| self.regiments[i])
    }

    fn extras(&self, id: RegimentId) -> Option<&RegimentExtras> {
        self.index(id).map(|i| &self.extras[i])
    }

    /// The regiment's composition in setup order (SIM-FORM-012).
    pub fn regiment_units(&self, id: RegimentId) -> &[UnitGroup] {
        self.extras(id).map_or(&[], |x| x.units.as_slice())
    }

    /// The formations the regiment may take (SIM-FORM-014).
    pub fn regiment_formations(&self, id: RegimentId) -> &[Handle<FormationTemplate>] {
        self.extras(id).map_or(&[], |x| x.formations.as_slice())
    }

    /// Living soldiers per group, in setup order.
    pub fn regiment_living_by_group(&self, id: RegimentId) -> &[u16] {
        self.extras(id)
            .map_or(&[], |x| x.living_by_group.as_slice())
    }

    pub fn abilities(&self, id: RegimentId) -> &[AbilityRow] {
        self.extras(id).map_or(&[], |x| x.abilities.as_slice())
    }

    pub fn statuses(&self, id: RegimentId) -> &[StatusRow] {
        self.extras(id).map_or(&[], |x| x.statuses.as_slice())
    }

    /// Volleys left (the most any soldier carries).
    pub fn ammo(&self, id: RegimentId) -> u16 {
        self.extras(id).map_or(0, |x| x.ammo)
    }

    /// SIM-VIS-001: the line-of-sight radius at the capture.
    pub fn los_radius(&self, id: RegimentId) -> S {
        self.extras(id).map_or(S::ZERO, |x| x.los_radius)
    }

    /// The formation state; `None` unless the frame was captured with
    /// `FrameDetail::formation`.
    pub fn formation_state(&self, id: RegimentId) -> Option<&FormationState> {
        self.extras(id).and_then(|x| x.formation.as_ref())
    }

    /// The current path; `None` unless captured with `FrameDetail::paths`.
    pub fn path(&self, id: RegimentId) -> Option<&Path> {
        self.extras(id).and_then(|x| x.path.as_ref())
    }

    /// SIM-VIS-004: whether `side` saw the regiment at the capture.
    pub fn visible(&self, side: u8, id: RegimentId) -> bool {
        let n = self.regiments.len();
        self.index(id)
            .and_then(|i| self.visible.get(usize::from(side) * n + i))
            .copied()
            .unwrap_or(false)
    }

    /// SIM-VIS-005: what `side` last saw of a regiment it does not see now.
    pub fn seen(&self, side: u8, id: RegimentId) -> Option<Seen> {
        let n = self.regiments.len();
        self.index(id)
            .and_then(|i| self.seen.get(usize::from(side) * n + i))
            .copied()
            .flatten()
    }

    /// The engine's plan for a side; `None` unless captured with
    /// `FrameDetail::ai`.
    pub fn ai_plan(&self, side: u8) -> Option<&ArmyPlan> {
        self.ai_plans.get(usize::from(side))?.as_ref()
    }

    /// The soldier grid; `None` unless captured with `FrameDetail::spatial`.
    pub fn spatial_grid(&self) -> Option<&SpatialGrid<SoldierId>> {
        self.spatial.as_ref()
    }

    /// Every projectile in flight, ascending id.
    pub fn projectiles(&self) -> impl Iterator<Item = ProjectileRow> + '_ {
        self.projectiles.iter().copied()
    }
}
