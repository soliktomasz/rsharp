//! The tool windows' frame, in IntelliJ's six places: a stripe down each side holds a window's button in the
//! place it sits (left or right, top or split below, or the bottom panel's left or right half), and an open
//! window is an island beside the editor. Where each window sits, which are open and the islands' sizes are a
//! `DockLayout`, kept with the folder and as the default for a folder with none. A floating window keeps its
//! stripe button and place, as IntelliJ's do, and is drawn over the frame where it was left.

use rsharp_core::state::StateDocument;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Place {
    LeftTop,
    LeftBottom,
    BottomLeft,
    RightTop,
    RightBottom,
    BottomRight,
}

impl Place {
    /// A stripe's three groups, top to bottom: the side's top place, its split below, then the bottom panel's half.
    pub const LEFT_STRIPE: [Place; 3] = [Place::LeftTop, Place::LeftBottom, Place::BottomLeft];
    pub const RIGHT_STRIPE: [Place; 3] = [Place::RightTop, Place::RightBottom, Place::BottomRight];

    pub fn name(self) -> &'static str {
        match self {
            Place::LeftTop => "Left Top",
            Place::LeftBottom => "Left Bottom",
            Place::BottomLeft => "Bottom Left",
            Place::RightTop => "Right Top",
            Place::RightBottom => "Right Bottom",
            Place::BottomRight => "Bottom Right",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Tool {
    Solution,
    Problems,
    /// Find Usages' results, and every other list of places to go.
    Find,
    NuGet,
    Build,
    Run,
    UnitTests,
    Terminal,
}

impl Tool {
    pub const ALL: [Tool; 8] = [
        Tool::Solution,
        Tool::Problems,
        Tool::Find,
        Tool::NuGet,
        Tool::Build,
        Tool::Run,
        Tool::UnitTests,
        Tool::Terminal,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Tool::Solution => "Solution",
            Tool::Problems => "Problems",
            Tool::Find => "Find",
            Tool::NuGet => "NuGet",
            Tool::Build => "Build",
            Tool::Run => "Run",
            Tool::UnitTests => "Unit Tests",
            Tool::Terminal => "Terminal",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Tool::Solution => "tw-project",
            Tool::Problems => "tw-problems",
            Tool::Find => "tw-find",
            Tool::NuGet => "package",
            Tool::Build => "tw-build",
            Tool::Run => "tw-run",
            Tool::UnitTests => "check",
            Tool::Terminal => "tw-terminal",
        }
    }

    /// Where `Dock.Add` would put it: Rider's place for it.
    fn home(self) -> Place {
        match self {
            Tool::Solution => Place::LeftTop,
            Tool::Problems => Place::BottomLeft,
            Tool::Find => Place::BottomLeft,
            Tool::NuGet => Place::BottomLeft,
            Tool::Build => Place::BottomLeft,
            Tool::Run => Place::BottomLeft,
            Tool::UnitTests => Place::BottomLeft,
            Tool::Terminal => Place::BottomLeft,
        }
    }
}

/// A floating window's bounds in the window, in pixels from its top left.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct DockLayout {
    /// Each window's place, in its stripe group's order.
    pub placed: Vec<(Tool, Place)>,
    pub open: Vec<Tool>,
    pub left_width: f32,
    pub right_width: f32,
    pub bottom_height: f32,
    pub floating: Vec<(Tool, Rect)>,
}

impl Default for DockLayout {
    fn default() -> Self {
        Self {
            placed: Tool::ALL.iter().map(|t| (*t, t.home())).collect(),
            open: vec![Tool::Solution],
            left_width: 280.,
            right_width: 280.,
            bottom_height: 220.,
            floating: Vec::new(),
        }
    }
}

/// The last layout a person left, which a folder without its own opens on (Passenger's `layout.json`).
impl StateDocument for DockLayout {
    const FILE: &'static str = "layout.json";
}

impl DockLayout {
    pub const MIN_SIDE: f32 = 160.;
    pub const MIN_BOTTOM: f32 = 100.;

    /// Every tool placed once: a window a newer version added, or a layout file lost, goes to its home.
    pub fn complete(mut self) -> Self {
        self.placed.retain(|(t, _)| Tool::ALL.contains(t));
        self.placed.dedup_by_key(|(t, _)| *t);
        for tool in Tool::ALL {
            if !self.placed.iter().any(|(t, _)| *t == tool) {
                self.placed.push((tool, tool.home()));
            }
        }
        self.open.dedup();
        self.floating.dedup_by_key(|(t, _)| *t);
        self
    }

    pub fn place_of(&self, tool: Tool) -> Place {
        self.placed.iter().find(|(t, _)| *t == tool).map(|(_, p)| *p).unwrap_or(tool.home())
    }

    pub fn in_place(&self, place: Place) -> impl Iterator<Item = Tool> + '_ {
        self.placed.iter().filter(move |(_, p)| *p == place).map(|(t, _)| *t)
    }

    /// The docked window open in a place: one at a time, as IntelliJ shows them.
    pub fn open_in(&self, place: Place) -> Option<Tool> {
        self.in_place(place).find(|t| self.open.contains(t) && !self.is_floating(*t))
    }

    pub fn floating_at(&self, tool: Tool) -> Option<Rect> {
        self.floating.iter().find(|(t, _)| *t == tool).map(|(_, r)| *r)
    }

    pub fn is_floating(&self, tool: Tool) -> bool {
        self.floating_at(tool).is_some()
    }

    /// The floating windows open, in the order they are drawn.
    pub fn open_floating(&self) -> impl Iterator<Item = (Tool, Rect)> + '_ {
        self.floating.iter().filter(|(t, _)| self.is_open(*t)).copied()
    }

    /// Floats a window at a place in the window, open; one floating already moves there.
    pub fn float(&mut self, tool: Tool, at: Rect) {
        self.floating.retain(|(t, _)| *t != tool);
        self.floating.push((tool, at));
        self.show(tool);
    }

    /// Docks a floating window back in its place, open there if it was open.
    pub fn dock(&mut self, tool: Tool) {
        self.floating.retain(|(t, _)| *t != tool);
        if self.is_open(tool) {
            self.show(tool);
        }
    }

    pub fn is_open(&self, tool: Tool) -> bool {
        self.open.contains(&tool)
    }

    /// Opens a window, closing the one that shared its place, or closes it when open.
    pub fn toggle(&mut self, tool: Tool) {
        if self.is_open(tool) {
            self.open.retain(|t| *t != tool);
        } else {
            self.show(tool);
        }
    }

    /// Opens a window; a docked one closes the docked window that shared its place.
    pub fn show(&mut self, tool: Tool) {
        let place = self.place_of(tool);
        let others: Vec<Tool> = if self.is_floating(tool) {
            Vec::new()
        } else {
            self.in_place(place).filter(|t| *t != tool && !self.is_floating(*t)).collect()
        };
        self.open.retain(|t| !others.contains(t));
        if !self.open.contains(&tool) {
            self.open.push(tool);
        }
    }

    /// Moves a window to a place, last in its group, docked and open there if it was open.
    pub fn move_to(&mut self, tool: Tool, place: Place) {
        self.placed.retain(|(t, _)| *t != tool);
        self.floating.retain(|(t, _)| *t != tool);
        self.placed.push((tool, place));
        if self.is_open(tool) {
            self.show(tool);
        }
    }

    pub fn clamped(mut self) -> Self {
        self.left_width = self.left_width.max(Self::MIN_SIDE);
        self.right_width = self.right_width.max(Self::MIN_SIDE);
        self.bottom_height = self.bottom_height.max(Self::MIN_BOTTOM);
        for (_, r) in &mut self.floating {
            r.x = r.x.max(0.);
            r.y = r.y.max(0.);
            r.w = r.w.max(Self::MIN_SIDE);
            r.h = r.h.max(Self::MIN_BOTTOM);
        }
        self
    }
}
