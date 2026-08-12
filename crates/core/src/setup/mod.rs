pub mod plan;
pub mod profile;

mod node_capacitor;

/// Every action name the setup surfaces advertise, for surfaces that must report the set rather
/// than restate it.
///
/// A union across profiles, which is one profile today. The parse stays per profile, so a name here
/// is accepted only by the profile that owns it: the advertised keyword says which names exist, and
/// `node_capacitor::Action::parse` says which are valid for the profile actually selected. Adding a
/// second profile means extending this union as well as the profile match in `profile.rs`, and
/// neither is compiler-bound today because that match carries a catch-all for an unknown profile.
pub fn advertised_action_names() -> Vec<&'static str> {
    node_capacitor::Action::ALL
        .iter()
        .map(|action| action.as_str())
        .collect()
}
