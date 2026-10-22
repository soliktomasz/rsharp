//! The tool windows' frame, in IntelliJ's six places: a stripe down each side holds a window's button in the
//! place it sits (left or right, top or split below, or the bottom panel's left or right half), and an open
//! window is an island beside the editor. Where each window sits, which are open and the islands' sizes are a
//! `DockLayout`, kept with the folder and as the default for a folder with none.

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
}

impl Tool {
    pub const ALL: [Tool; 2] = [Tool::Solution, Tool::Problems];

    pub fn title(self) -> &'static str {
        match self {
            Tool::Solution => "Solution",
            Tool::Problems => "Problems",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Tool::Solution => "tw-project",
            Tool::Problems => "tw-problems",
        }
    }

    /// Where `Dock.Add` would put it: Rider's place for it.
    fn home(self) -> Place {
        match self {
            Tool::Solution => Place::LeftTop,
            Tool::Problems => Place::BottomLeft,
        }
    }
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
}

impl Default for DockLayout {
    fn default() -> Self {
        Self {
            placed: Tool::ALL.iter().map(|t| (*t, t.home())).collect(),
            open: vec![Tool::Solution],
            left_width: 280.,
            right_width: 280.,
            bottom_height: 220.,
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
        self
    }

    pub fn place_of(&self, tool: Tool) -> Place {
        self.placed.iter().find(|(t, _)| *t == tool).map(|(_, p)| *p).unwrap_or(tool.home())
    }

    pub fn in_place(&self, place: Place) -> impl Iterator<Item = Tool> + '_ {
        self.placed.iter().filter(move |(_, p)| *p == place).map(|(t, _)| *t)
    }

    /// The window open in a place: one at a time, as IntelliJ shows them.
    pub fn open_in(&self, place: Place) -> Option<Tool> {
        self.in_place(place).find(|t| self.open.contains(t))
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

    pub fn show(&mut self, tool: Tool) {
        let place = self.place_of(tool);
        let others: Vec<Tool> = self.in_place(place).filter(|t| *t != tool).collect();
        self.open.retain(|t| !others.contains(t));
        if !self.open.contains(&tool) {
            self.open.push(tool);
        }
    }

    /// Moves a window to a place, last in its group, open there if it was open.
    pub fn move_to(&mut self, tool: Tool, place: Place) {
        self.placed.retain(|(t, _)| *t != tool);
        self.placed.push((tool, place));
        if self.is_open(tool) {
            self.show(tool);
        }
    }

    pub fn clamped(mut self) -> Self {
        self.left_width = self.left_width.max(Self::MIN_SIDE);
        self.right_width = self.right_width.max(Self::MIN_SIDE);
        self.bottom_height = self.bottom_height.max(Self::MIN_BOTTOM);
        self
    }
}
