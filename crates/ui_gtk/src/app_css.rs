pub(crate) fn install_app_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(include_str!("app.css"));

    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
