//! Cheat codes (our design, 08-missing-features §8.14): letters typed in the pause menu toggle a
//! gameplay change until the game is closed. The original has none that we know of.

/// A cheat code's effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cheat {
    /// the player's car is drawn like the Time Attack ghost
    Casper,
    /// all cars are scaled down to 20 %
    Gulliver,
}

/// The codes, lower case.
pub const CODES: [(&str, Cheat); 2] = [("casper", Cheat::Casper), ("gulliver", Cheat::Gulliver)];

/// Letters kept from the typing (the longest code is 7).
const KEPT: usize = 16;

/// The original's driver/team name cheats (motor-sim `cheats`): a separate state from the pause-menu
/// codes above, kept on the race (`Race::name_cheat`) and rebuilt for every race.
#[allow(unused_imports)] // the effect tickets (#310, #313-#321) are the callers
pub use motor_sim::cheats::{NameCheat, ninja_marker, unlocks_all};

/// The size (and handling feel) of every car while `gulliver` is on (motor-sim `Race::set_car_scale`).
pub use motor_sim::session::{GULLIVER_FEEL, GULLIVER_SCALE};

#[derive(Default)]
pub struct Cheats {
    pub casper: bool,
    pub gulliver: bool,
    typed: String,
}

impl Cheats {
    fn flag(&mut self, c: Cheat) -> &mut bool {
        match c {
            Cheat::Casper => &mut self.casper,
            Cheat::Gulliver => &mut self.gulliver,
        }
    }

    /// Letters typed (other characters are dropped). A code at the end of the typing toggles its
    /// cheat and clears the typing: returns it and whether it is now on.
    pub fn type_text(&mut self, t: &str) -> Option<(Cheat, bool)> {
        self.typed.extend(t.chars().filter(char::is_ascii_alphabetic).map(|c| c.to_ascii_lowercase()));
        if self.typed.len() > KEPT {
            self.typed.drain(..self.typed.len() - KEPT);
        }
        let &(_, cheat) = CODES.iter().find(|(code, _)| self.typed.ends_with(code))?;
        self.typed.clear();
        let on = self.flag(cheat);
        *on ^= true;
        Some((cheat, *on))
    }

    /// Whether the typing so far ends in the start (two letters or more) of a code: the pause
    /// menu's E (tab key) then belongs to the code ("casper").
    pub fn mid_code(&self) -> bool {
        CODES.iter().any(|(code, _)| (2..code.len()).any(|n| self.typed.ends_with(&code[..n])))
    }

    /// Size and handling feel of every car for `Race::set_car_scale`: (1.0, 1.0) off.
    pub fn car_size(&self) -> (f32, f32) {
        if self.gulliver { (GULLIVER_SCALE, GULLIVER_FEEL) } else { (1.0, 1.0) }
    }

    /// Turn the cheats named in a comma separated list on (`--cheat`); the unknown names.
    pub fn set_by_names(&mut self, list: &str) -> Result<(), String> {
        for n in list.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            let &(_, cheat) = CODES.iter().find(|(code, _)| code.eq_ignore_ascii_case(n)).ok_or(format!("unknown cheat {n}"))?;
            *self.flag(cheat) = true;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mid_code_tells_a_code_from_a_tab_key() {
        let mut c = Cheats::default();
        c.type_text("e");
        assert!(!c.mid_code());
        c.type_text("casp");
        c.type_text("e");
        assert!(c.mid_code());
    }

    #[test]
    fn a_code_toggles_its_cheat() {
        let mut c = Cheats::default();
        assert_eq!(c.type_text("casper"), Some((Cheat::Casper, true)));
        assert!(c.casper && !c.gulliver);
        assert_eq!(c.type_text("casper"), Some((Cheat::Casper, false)));
        assert!(!c.casper);
    }

    #[test]
    fn gulliver_shrinks_and_a_second_typing_restores() {
        let mut c = Cheats::default();
        assert_eq!(c.car_size(), (1.0, 1.0));
        c.type_text("gulliver");
        assert_eq!(c.car_size(), (GULLIVER_SCALE, GULLIVER_FEEL));
        assert_eq!(c.type_text("gulliver"), Some((Cheat::Gulliver, false)));
        assert_eq!(c.car_size(), (1.0, 1.0));
    }

    #[test]
    fn typing_in_pieces_and_around_the_code_counts() {
        let mut c = Cheats::default();
        assert_eq!(c.type_text("xx Gu"), None);
        assert_eq!(c.type_text("LLI"), None);
        assert_eq!(c.type_text("ver"), Some((Cheat::Gulliver, true)));
        assert!(c.gulliver);
    }

    #[test]
    fn a_partial_code_does_nothing() {
        let mut c = Cheats::default();
        assert_eq!(c.type_text("caspe"), None);
        assert_eq!(c.type_text("1 2"), None);
        assert!(!c.casper);
    }

    #[test]
    fn names_for_the_command_line() {
        let mut c = Cheats::default();
        assert!(c.set_by_names("casper, GULLIVER").is_ok());
        assert!(c.casper && c.gulliver);
        assert!(c.set_by_names("nope").is_err());
    }
}
