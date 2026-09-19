mod app;
mod protocol;
mod reader;
mod token;

fn main() {
    leptos::mount::mount_to_body(app::App);
}
