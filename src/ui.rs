use cosmic::prelude::*;

fn button_style(theme: &cosmic::Theme, alpha: f32) -> cosmic::widget::button::Style {
    let cosmic = theme.cosmic();
    let mut style = cosmic::widget::button::Style::new();
    if alpha > 0.0 {
        style.background = Some(cosmic::iced::Background::Color(cosmic::iced::Color {
            a: alpha,
            ..cosmic.accent_color().into()
        }));
        style.border_radius = cosmic.corner_radii.radius_s.into();
    }
    style
}

pub fn icon_button_style(alpha: f32) -> cosmic::theme::Button {
    cosmic::theme::Button::Custom {
        active: Box::new(move |_focused, theme| button_style(theme, alpha)),
        disabled: Box::new(move |theme| button_style(theme, alpha)),
        hovered: Box::new(move |_focused, theme| {
            button_style(theme, (alpha + 0.10).clamp(0.20, 0.50))
        }),
        pressed: Box::new(move |_focused, theme| {
            button_style(theme, (alpha + 0.20).clamp(0.30, 0.60))
        }),
    }
}

pub fn divider<Message: 'static>(
    is_horizontal: bool,
    pad_x: f32,
    pad_y: f32,
) -> Element<'static, Message> {
    if is_horizontal {
        cosmic::widget::container(cosmic::widget::divider::vertical::default())
            .padding([pad_y as u16, pad_x as u16])
            .into()
    } else {
        cosmic::widget::container(cosmic::widget::divider::horizontal::default())
            .padding([pad_x as u16, pad_y as u16])
            .into()
    }
}

/// Formats an RFC 3339 timestamp relative to a caller-supplied instant.
pub fn format_relative_time(value: &str, now: jiff::Timestamp) -> String {
    let Ok(timestamp) = value.parse::<jiff::Timestamp>() else {
        return value.to_string();
    };
    let seconds = now.as_second().saturating_sub(timestamp.as_second()).max(0) as u64;
    match seconds {
        0..60 => "just now".to_string(),
        60..3_600 => format!("{} min ago", seconds / 60),
        3_600..86_400 => format!("{}h ago", seconds / 3_600),
        86_400..2_592_000 => {
            let days = seconds / 86_400;
            format!("{days} day{} ago", if days == 1 { "" } else { "s" })
        }
        2_592_000..31_536_000 => {
            let months = seconds / 2_592_000;
            format!("{months} month{} ago", if months == 1 { "" } else { "s" })
        }
        _ => {
            let years = seconds / 31_536_000;
            format!("{years} year{} ago", if years == 1 { "" } else { "s" })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::format_relative_time;

    #[test]
    fn relative_time_uses_compact_units() {
        let now: jiff::Timestamp = "2026-08-11T12:00:00Z".parse().unwrap();
        assert_eq!(
            format_relative_time("2026-08-11T11:59:00Z", now),
            "1 min ago"
        );
        assert_eq!(format_relative_time("2026-08-11T10:00:00Z", now), "2h ago");
        assert_eq!(
            format_relative_time("2026-08-09T12:00:00Z", now),
            "2 days ago"
        );
        assert_eq!(format_relative_time("invalid", now), "invalid");
    }
}
