// SPDX-License-Identifier: GPL-3.0-only

mod config;
mod i18n;
mod ui;

fn main() -> cosmic::iced::Result {
    let requested_languages = i18n_embed::DesktopLanguageRequester::requested_languages();
    i18n::init(&requested_languages);

    let settings = cosmic::app::Settings::default()
        .size(cosmic::iced::Size::new(960.0, 640.0))
        .size_limits(
            cosmic::iced::Limits::NONE
                .min_width(360.0)
                .min_height(240.0),
        );

    cosmic::app::run_single_instance::<ui::App>(settings, ui::Flags)
}
