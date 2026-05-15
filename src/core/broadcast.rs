use crate::lobby_handler::Target;
use crate::messages::Console;
use crate::session::SessionManager;

/// Who receives a broadcast message.
#[derive(Clone, Debug, PartialEq)]
pub enum Audience {
    All,
    Holding(Console),
    Token(String),
    AllExcept(String),
}

impl Audience {
    /// Resolve this audience to a `Target` given current session state.
    /// Returns `None` when `Holding` names a console with no current holder,
    /// signalling the caller to skip this broadcast.
    pub fn resolve(&self, sessions: &SessionManager) -> Option<Target> {
        match self {
            Audience::All => Some(Target::All),
            Audience::Holding(console) => sessions
                .console_holder(console.clone())
                .map(|t| Target::Token(t.to_string())),
            Audience::Token(t) => Some(Target::Token(t.clone())),
            Audience::AllExcept(t) => Some(Target::AllExcept(t.clone())),
        }
    }
}

/// How often a broadcast registration fires.
#[derive(Clone, Debug)]
pub enum Cadence {
    Hz(f32),
    Period(std::time::Duration),
    OnEvent,
    Once,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sm_with_holder(token: &str, console: Console) -> SessionManager {
        let mut sm = SessionManager::new();
        sm.register(token.to_string(), "Player".to_string()).unwrap();
        sm.toggle_console(token, console).unwrap();
        sm
    }

    #[test]
    fn audience_all_always_resolves() {
        let sm = SessionManager::new();
        assert_eq!(Audience::All.resolve(&sm), Some(Target::All));
    }

    #[test]
    fn audience_holding_returns_none_when_no_holder() {
        let sm = SessionManager::new();
        assert_eq!(Audience::Holding(Console::Power).resolve(&sm), None);
    }

    #[test]
    fn audience_holding_returns_token_when_console_held() {
        let sm = sm_with_holder("tok1", Console::Power);
        assert_eq!(
            Audience::Holding(Console::Power).resolve(&sm),
            Some(Target::Token("tok1".to_string()))
        );
    }

    #[test]
    fn audience_holding_returns_none_for_different_console() {
        let sm = sm_with_holder("tok1", Console::Power);
        assert_eq!(Audience::Holding(Console::Helm).resolve(&sm), None);
    }

    #[test]
    fn audience_token_always_resolves() {
        let sm = SessionManager::new();
        assert_eq!(
            Audience::Token("abc".to_string()).resolve(&sm),
            Some(Target::Token("abc".to_string()))
        );
    }

    #[test]
    fn audience_all_except_always_resolves() {
        let sm = SessionManager::new();
        assert_eq!(
            Audience::AllExcept("abc".to_string()).resolve(&sm),
            Some(Target::AllExcept("abc".to_string()))
        );
    }
}
