#![forbid(unsafe_code)]

fn main() {
    let runtime = xtunes_app_runtime::ApplicationRuntime::new();
    xtunes_ui_gtk::run(runtime);
}
