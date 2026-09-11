//! The custom battle builder (T2-091, REQ-UI-007, REQ-SIM-063; plan
//! decision 12, I16, I17): a map from the registry (weather limited to its
//! `weather_allowed`, a seed, a time limit), two to four sides, each with a
//! faction, a controller (you, the engine AI, an idle player), a general and
//! roster rows of the faction's units. `to_setup` turns the draft into a
//! positionless `BattleSetup` (so the battle opens in Deployment) or names
//! what is wrong. The app writes the JSON5 file on Save.

use il_core::PlayerId;
use il_data::{ContentId, Locale, UnitCategory};
use il_sim_battle::{
    BattleSetup, GeneralSetup, RegimentSetup, SOLDIER_CAP, SideSetup, VictoryRules, Weather,
};

/// Roster limits (decision 12).
pub const MIN_SIDES: usize = 2;
pub const MAX_SIDES: usize = 4;
pub const MAX_COUNT: u16 = 1000;
pub const MAX_EXPERIENCE: u8 = 9;
pub const TIME_LIMIT_MINUTES: (u32, u32) = (1, 120);
/// Ticks per minute of battle time.
pub const TICKS_PER_MINUTE: u32 = 1200;
/// The remove-row button (a minus sign, not a word).
const MINUS: &str = "\u{2212}";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitChoice {
    pub id: ContentId,
    pub name: String,
    pub category: UnitCategory,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FactionChoice {
    pub id: ContentId,
    pub name: String,
    /// The faction's units that are not generals.
    pub units: Vec<UnitChoice>,
    /// The faction's general-category units (every general in the registry
    /// when it lists none, plan I16).
    pub generals: Vec<UnitChoice>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapChoice {
    pub id: ContentId,
    pub name: String,
    /// Deployment polygons: the most sides the map takes.
    pub zones: usize,
    /// `weather_allowed`, lower-case names.
    pub weather: Vec<String>,
}

/// What the builder offers, built by the app from the registries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuilderCatalog {
    pub maps: Vec<MapChoice>,
    pub factions: Vec<FactionChoice>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Controller {
    You,
    EngineAi,
    Idle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowDraft {
    /// Index into the faction's `units`.
    pub unit: usize,
    pub count: u16,
    pub experience: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SideDraft {
    /// Index into the catalog's factions.
    pub faction: usize,
    pub controller: Controller,
    /// Index into the faction's `generals`.
    pub general: usize,
    pub rows: Vec<RowDraft>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BuilderState {
    pub map: usize,
    pub seed: u64,
    /// Index into the map's weather list.
    pub weather: usize,
    pub time_limit_minutes: u32,
    pub sides: Vec<SideDraft>,
    /// The scenario file's stem for Save.
    pub name: String,
    /// The last failure (localised), shown under the buttons.
    pub error: Option<String>,
    /// Save asked once over an existing file; the next click overwrites.
    pub confirm_overwrite: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuilderAction {
    Start,
    Save,
    /// Draw a fresh seed (the app owns the clock).
    RandomSeed,
    Back,
}

/// Why a draft is not a battle (`il.custom.err.*`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildError {
    NoMap,
    TooFewSides,
    TooManySides { zones: usize },
    SeveralYou,
    EmptySide { side: usize },
    BadCount { side: usize },
    OverCap { soldiers: u32 },
    NoGeneral { side: usize },
}

impl BuildError {
    pub fn key(&self) -> &'static str {
        match self {
            BuildError::NoMap => "il.custom.err.no_map",
            BuildError::TooFewSides => "il.custom.err.too_few_sides",
            BuildError::TooManySides { .. } => "il.custom.err.too_many_sides",
            BuildError::SeveralYou => "il.custom.err.several_you",
            BuildError::EmptySide { .. } => "il.custom.err.empty_side",
            BuildError::BadCount { .. } => "il.custom.err.bad_count",
            BuildError::OverCap { .. } => "il.custom.err.over_cap",
            BuildError::NoGeneral { .. } => "il.custom.err.no_general",
        }
    }

    pub fn text(&self, l: &Locale) -> String {
        match self {
            BuildError::TooManySides { zones } => l.fmt(self.key(), &[("zones", zones)]),
            BuildError::EmptySide { side }
            | BuildError::BadCount { side }
            | BuildError::NoGeneral { side } => l.fmt(self.key(), &[("side", &(side + 1))]),
            BuildError::OverCap { soldiers } => {
                l.fmt(self.key(), &[("soldiers", soldiers), ("cap", &SOLDIER_CAP)])
            }
            _ => l.get(self.key()).to_string(),
        }
    }
}

fn weather_from(name: &str) -> Weather {
    match name {
        "rain" => Weather::Rain,
        "fog" => Weather::Fog,
        _ => Weather::Clear,
    }
}

impl BuilderState {
    /// Two sides: you with the first faction, the engine with the second (or
    /// the first again), one row of the faction's first unit each.
    pub fn default_for(catalog: &BuilderCatalog, seed: u64) -> Self {
        let side = |faction: usize, controller: Controller| SideDraft {
            faction,
            controller,
            general: 0,
            rows: vec![RowDraft {
                unit: 0,
                count: 120,
                experience: 0,
            }],
        };
        let second = usize::from(catalog.factions.len() > 1);
        Self {
            map: 0,
            seed,
            weather: 0,
            time_limit_minutes: 40,
            sides: vec![side(0, Controller::You), side(second, Controller::EngineAi)],
            name: format!("custom_{seed}"),
            error: None,
            confirm_overwrite: false,
        }
    }

    /// Soldiers in the draft, generals included.
    pub fn soldiers(&self) -> u32 {
        self.sides
            .iter()
            .map(|s| 1 + s.rows.iter().map(|r| u32::from(r.count)).sum::<u32>())
            .sum()
    }

    /// The `BattleSetup` for the draft (plan I16): you are player 0, idle
    /// sides 1, 2, …, the engine's sides player 255 (decision 12);
    /// deployment zone = side index; no positions, so Deployment opens.
    pub fn to_setup(&self, catalog: &BuilderCatalog) -> Result<BattleSetup, BuildError> {
        let map = catalog.maps.get(self.map).ok_or(BuildError::NoMap)?;
        if self.sides.len() < MIN_SIDES {
            return Err(BuildError::TooFewSides);
        }
        if self.sides.len() > MAX_SIDES.min(map.zones) {
            return Err(BuildError::TooManySides { zones: map.zones });
        }
        if self
            .sides
            .iter()
            .filter(|s| s.controller == Controller::You)
            .count()
            > 1
        {
            return Err(BuildError::SeveralYou);
        }
        let soldiers = self.soldiers();
        if soldiers > SOLDIER_CAP {
            return Err(BuildError::OverCap { soldiers });
        }
        let mut next_idle = 1u8;
        let mut next_id = 1u32;
        let mut sides = Vec::new();
        for (i, s) in self.sides.iter().enumerate() {
            let faction = catalog.factions.get(s.faction).ok_or(BuildError::NoMap)?;
            if s.rows.is_empty() {
                return Err(BuildError::EmptySide { side: i });
            }
            let general = faction
                .generals
                .get(s.general)
                .ok_or(BuildError::NoGeneral { side: i })?;
            let player = match s.controller {
                Controller::You => PlayerId(0),
                Controller::EngineAi => PlayerId::ENGINE_AI,
                Controller::Idle => {
                    let p = PlayerId(next_idle);
                    next_idle += 1;
                    p
                }
            };
            let mut regiments = Vec::new();
            for r in &s.rows {
                if r.count == 0 || r.count > MAX_COUNT {
                    return Err(BuildError::BadCount { side: i });
                }
                let unit = faction
                    .units
                    .get(r.unit)
                    .ok_or(BuildError::EmptySide { side: i })?;
                regiments.push(RegimentSetup {
                    experience: Some(r.experience.min(MAX_EXPERIENCE)),
                    formation: None,
                    position: None,
                    facing_deg: None,
                    ..RegimentSetup::single(next_id, unit.id.clone(), r.count)
                });
                next_id += 1;
            }
            sides.push(SideSetup {
                faction: faction.id.clone(),
                player,
                deployment_zone: i as u8,
                general: GeneralSetup {
                    unit_type: general.id.clone(),
                    rank: 1,
                    name_key: String::new(),
                    bodyguard: None,
                },
                regiments,
                reinforcements: Vec::new(),
                ai_profile: None,
            });
        }
        Ok(BattleSetup {
            map_id: map.id.clone(),
            seed: self.seed,
            weather: map
                .weather
                .get(self.weather)
                .map_or(Weather::Clear, |w| weather_from(w)),
            time_of_day: 12,
            time_limit_ticks: self
                .time_limit_minutes
                .clamp(TIME_LIMIT_MINUTES.0, TIME_LIMIT_MINUTES.1)
                * TICKS_PER_MINUTE,
            reveal_deployment: false,
            sides,
            victory: VictoryRules::default(),
        })
    }
}

fn combo<T>(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    index: &mut usize,
    items: &[T],
    label: impl Fn(&T) -> String,
) -> bool {
    let mut changed = false;
    let current = items.get(*index).map(&label).unwrap_or_default();
    egui::ComboBox::from_id_salt(id)
        .selected_text(current)
        .show_ui(ui, |ui| {
            for (i, item) in items.iter().enumerate() {
                if ui.selectable_label(i == *index, label(item)).clicked() && i != *index {
                    *index = i;
                    changed = true;
                }
            }
        });
    changed
}

/// Draws the builder; returns the click, if any.
pub fn custom_battle(
    ctx: &egui::Context,
    state: &mut BuilderState,
    catalog: &BuilderCatalog,
    locale: &Locale,
) -> Option<BuilderAction> {
    let l = locale;
    let mut action = None;
    egui::Window::new("il_custom_battle")
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 24.0))
        .title_bar(false)
        .resizable(true)
        .default_width(760.0)
        .show(ctx, |ui| {
            ui.heading(l.get("il.custom.title"));
            ui.add_space(8.0);
            egui::Grid::new("il_custom_top")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label(l.get("il.custom.map"));
                    if combo(ui, "il_custom_map", &mut state.map, &catalog.maps, |m| {
                        m.name.clone()
                    }) {
                        state.weather = 0;
                    }
                    ui.end_row();
                    ui.label(l.get("il.custom.weather"));
                    let weather: Vec<String> = catalog
                        .maps
                        .get(state.map)
                        .map(|m| {
                            m.weather
                                .iter()
                                .map(|w| l.get(&format!("il.weather.{w}")).to_string())
                                .collect()
                        })
                        .unwrap_or_default();
                    combo(ui, "il_custom_weather", &mut state.weather, &weather, |w| {
                        w.clone()
                    });
                    ui.end_row();
                    ui.label(l.get("il.custom.seed"));
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut state.seed).speed(1.0));
                        if ui.button(l.get("il.custom.random")).clicked() {
                            action = Some(BuilderAction::RandomSeed);
                        }
                    });
                    ui.end_row();
                    ui.label(l.get("il.custom.time_limit"));
                    ui.add(
                        egui::Slider::new(
                            &mut state.time_limit_minutes,
                            TIME_LIMIT_MINUTES.0..=TIME_LIMIT_MINUTES.1,
                        )
                        .suffix(l.get("il.custom.minutes")),
                    );
                    ui.end_row();
                });
            ui.separator();
            let mut remove_side = None;
            let n_sides = state.sides.len();
            for (i, side) in state.sides.iter_mut().enumerate() {
                let faction = catalog.factions.get(side.faction);
                ui.push_id(i, |ui| {
                    ui.horizontal(|ui| {
                        ui.strong(l.fmt("il.custom.side", &[("n", &(i + 1))]));
                        if combo(ui, "faction", &mut side.faction, &catalog.factions, |f| {
                            f.name.clone()
                        }) {
                            side.general = 0;
                            for r in &mut side.rows {
                                r.unit = 0;
                            }
                        }
                        for (c, key) in [
                            (Controller::You, "il.custom.you"),
                            (Controller::EngineAi, "il.custom.engine"),
                            (Controller::Idle, "il.custom.idle"),
                        ] {
                            if ui
                                .selectable_label(side.controller == c, l.get(key))
                                .clicked()
                            {
                                side.controller = c;
                            }
                        }
                        if let Some(f) = faction {
                            ui.label(l.get("il.custom.general"));
                            combo(ui, "general", &mut side.general, &f.generals, |g| {
                                g.name.clone()
                            });
                        }
                        if n_sides > MIN_SIDES
                            && ui.small_button(l.get("il.custom.remove_side")).clicked()
                        {
                            remove_side = Some(i);
                        }
                    });
                    let mut remove_row = None;
                    let n_rows = side.rows.len();
                    egui::Grid::new("rows").num_columns(4).show(ui, |ui| {
                        for (k, row) in side.rows.iter_mut().enumerate() {
                            ui.push_id(k, |ui| {
                                if let Some(f) = faction {
                                    combo(ui, "unit", &mut row.unit, &f.units, |u| u.name.clone());
                                }
                            });
                            ui.add(
                                egui::DragValue::new(&mut row.count)
                                    .range(1..=MAX_COUNT)
                                    .suffix(l.get("il.custom.soldiers")),
                            );
                            ui.add(
                                egui::DragValue::new(&mut row.experience)
                                    .range(0..=MAX_EXPERIENCE)
                                    .prefix(l.get("il.custom.experience")),
                            );
                            if n_rows > 1 && ui.small_button(MINUS).clicked() {
                                remove_row = Some(k);
                            }
                            ui.end_row();
                        }
                    });
                    if let Some(k) = remove_row {
                        side.rows.remove(k);
                    }
                    if ui.small_button(l.get("il.custom.add_row")).clicked() {
                        side.rows.push(RowDraft {
                            unit: 0,
                            count: 120,
                            experience: 0,
                        });
                    }
                    ui.separator();
                });
            }
            if let Some(i) = remove_side {
                state.sides.remove(i);
            }
            let zones = catalog.maps.get(state.map).map_or(0, |m| m.zones);
            if state.sides.len() < MAX_SIDES.min(zones)
                && ui.button(l.get("il.custom.add_side")).clicked()
            {
                let faction = state.sides.len() % catalog.factions.len().max(1);
                state.sides.push(SideDraft {
                    faction,
                    controller: Controller::EngineAi,
                    general: 0,
                    rows: vec![RowDraft {
                        unit: 0,
                        count: 120,
                        experience: 0,
                    }],
                });
            }
            ui.label(l.fmt(
                "il.custom.total",
                &[("soldiers", &state.soldiers()), ("cap", &SOLDIER_CAP)],
            ));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(l.get("il.custom.start")).clicked() {
                    action = Some(BuilderAction::Start);
                }
                ui.label(l.get("il.custom.name"));
                ui.text_edit_singleline(&mut state.name);
                let save = if state.confirm_overwrite {
                    l.get("il.custom.overwrite")
                } else {
                    l.get("il.custom.save")
                };
                if ui.button(save).clicked() {
                    action = Some(BuilderAction::Save);
                }
                if ui.button(l.get("il.menu.back")).clicked() {
                    action = Some(BuilderAction::Back);
                }
            });
            if let Some(e) = &state.error {
                ui.colored_label(egui::Color32::from_rgb(255, 120, 120), e);
            }
        });
    action
}
