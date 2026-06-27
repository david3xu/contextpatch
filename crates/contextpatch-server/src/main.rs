mod protocol;
mod tools;

fn main() {
    eprintln!(
        "{} server: not implemented yet; planned tools: {}",
        protocol::schema::PROTOCOL_NAME,
        protocol::tools::TOOL_NAMES.join(", ")
    );
    let _registered_tool_names = [
        tools::read_range::NAME,
        tools::diff_preview::NAME,
        tools::replace_exact::NAME,
        tools::apply_patch::NAME,
        tools::status_guard::NAME,
    ];
    std::process::exit(2);
}
