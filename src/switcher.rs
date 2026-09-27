//! 映像のルーティング。どのソースをPreview・Program・Input 1..8に出すかを持つ。

use std::fmt;
use std::ops::{Index, IndexMut};

use crate::source::SourceId;

/// Inputスロットの数。
pub const INPUTS: usize = 8;

/// 映像ソースを割り当てられる場所。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    Preview,
    Program,
    /// 0始まりの番号(表示は1始まり)
    Input(usize),
}

impl Slot {
    /// 全スロット。この順番が、1つのソースが複数に出ているときの表示の優先順でもある。
    pub fn all() -> impl Iterator<Item = Self> {
        [Self::Preview, Self::Program]
            .into_iter()
            .chain((0..INPUTS).map(Self::Input))
    }
}

/// 短い表示名(`PVW` `PGM` `IN 1` ...)。
impl fmt::Display for Slot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Preview => f.write_str("PVW"),
            Self::Program => f.write_str("PGM"),
            Self::Input(index) => write!(f, "IN {}", index + 1),
        }
    }
}

/// スロットへの割り当て。`switcher[Slot::Preview] = Some(id)` のように読み書きする。
#[derive(Debug, Default)]
pub struct Switcher {
    preview: Option<SourceId>,
    program: Option<SourceId>,
    inputs: [Option<SourceId>; INPUTS],
}

impl Switcher {
    /// `id` が出ているスロット。複数に出ていれば [`Slot::all`] の順で最初のもの。
    pub fn slot_of(&self, id: &SourceId) -> Option<Slot> {
        Slot::all().find(|&slot| self[slot].as_ref() == Some(id))
    }

    /// どこかに割り当てられているソース(同じものが複数回出ることがある)。
    pub fn sources(&self) -> impl Iterator<Item = &SourceId> {
        Slot::all().filter_map(|slot| self[slot].as_ref())
    }
}

impl Index<Slot> for Switcher {
    type Output = Option<SourceId>;

    fn index(&self, slot: Slot) -> &Self::Output {
        match slot {
            Slot::Preview => &self.preview,
            Slot::Program => &self.program,
            Slot::Input(index) => &self.inputs[index],
        }
    }
}

impl IndexMut<Slot> for Switcher {
    fn index_mut(&mut self, slot: Slot) -> &mut Self::Output {
        match slot {
            Slot::Preview => &mut self.preview,
            Slot::Program => &mut self.program,
            Slot::Input(index) => &mut self.inputs[index],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u32) -> SourceId {
        SourceId::new("test", n)
    }

    #[test]
    fn slot_of_prefers_preview_then_program_then_inputs() {
        let mut switcher = Switcher::default();
        switcher[Slot::Input(3)] = Some(id(1));
        assert_eq!(switcher.slot_of(&id(1)), Some(Slot::Input(3)));

        switcher[Slot::Program] = Some(id(1));
        assert_eq!(switcher.slot_of(&id(1)), Some(Slot::Program));

        switcher[Slot::Preview] = Some(id(1));
        assert_eq!(switcher.slot_of(&id(1)), Some(Slot::Preview));

        assert_eq!(switcher.slot_of(&id(2)), None);
    }

    #[test]
    fn sources_lists_assigned_slots() {
        let mut switcher = Switcher::default();
        switcher[Slot::Program] = Some(id(1));
        switcher[Slot::Input(0)] = Some(id(2));
        assert_eq!(switcher.sources().collect::<Vec<_>>(), [&id(1), &id(2)]);
    }

    #[test]
    fn slots_display_short_names() {
        let names: Vec<String> = Slot::all().map(|slot| slot.to_string()).collect();
        assert_eq!(names[..3], ["PVW", "PGM", "IN 1"]);
        assert_eq!(names.last().unwrap(), "IN 8");
    }
}
