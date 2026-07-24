use crate::niri::{Connection, Event};
use cosmic::iced::widget::{Column, Row};
use cosmic::iced::{window::Id, Subscription, Task};
use cosmic::prelude::*;
use cosmic::widget;
use cosmic::Application;
use niri_ipc::{Action, Reply, Request, Response, Window, Workspace};
use std::collections::HashMap;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct WindowView {
    pub id: u64,
    pub is_focused: bool,
    pub title: String,
    pub app_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceView {
    #[allow(dead_code)]
    pub id: u64,
    pub idx: u8,
    pub name: Option<String>,
    pub windows: Vec<WindowView>,
}

#[derive(Debug, Clone)]
pub struct DesktopAppInfo {
    pub path: std::path::PathBuf,
    pub name: String,
}

fn get_pinned_config_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("cosmic-ext-niri-windows").join("pinned.json"))
}

fn load_pinned() -> Vec<String> {
    if let Some(path) = get_pinned_config_path() {
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(pinned) = serde_json::from_str::<Vec<String>>(&content) {
                    return pinned;
                }
            }
        }
    }
    Vec::new()
}

fn save_pinned_async(pinned: Vec<String>) -> Task<Message> {
    Task::perform(
        async move {
            if let Some(path) = get_pinned_config_path() {
                if let Some(parent) = path.parent() {
                    let _ = tokio::fs::create_dir_all(parent).await;
                }
                if let Ok(content) = serde_json::to_string(&pinned) {
                    let _ = tokio::fs::write(&path, content).await;
                }
            }
        },
        |_| Message::Hover(None),
    )
}

#[derive(Debug, Clone, PartialEq)]
pub enum MenuTarget {
    Applet,
    Window { id: u64, app_id: Option<String> },
    Pinned { app_id: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum DraggedItem {
    Pinned { app_id: String, index: usize },
    Window { window_id: u64, workspace_id: u64 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum DragTarget {
    Window(u64),
    WorkspaceStart,
    WorkspaceEnd,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HoverTarget {
    Pinned(String),
    Window(u64),
}

pub struct AppModel {
    core: cosmic::Core,
    raw_windows: HashMap<u64, Window>,
    raw_workspaces: HashMap<u64, Workspace>,
    display: Vec<WorkspaceView>,
    last_scroll_time: Instant,
    action_tx: Option<cosmic::iced::futures::channel::mpsc::Sender<niri_ipc::Action>>,
    app_icon_cache: HashMap<String, widget::icon::Handle>,
    resolving_icons: std::collections::HashSet<String>,
    last_focused_window: Option<u64>,
    show_workspace_name: bool,
    context_menu_id: Option<Id>,
    last_mouse_pos: cosmic::iced::Point,
    pinned: Vec<String>,
    desktop_info_cache: HashMap<String, DesktopAppInfo>,
    hovered_target: Option<HoverTarget>,
    context_menu_target: Option<MenuTarget>,
    ignore_next_applet_right_click: bool,
    dragged_item: Option<DraggedItem>,
    has_drag_moved: bool,
    drag_x_start: f32,
}

#[derive(Debug, Clone)]
pub enum Message {
    #[allow(dead_code)]
    PopupClosed(#[allow(dead_code)] Id),
    NiriEvent(Event),
    InitialData {
        wins: Vec<Window>,
        wksps: Vec<Workspace>,
    },
    IconResolved {
        app_id: String,
        handle: widget::icon::Handle,
    },
    FocusWindow(u64),
    CloseWindow(u64),
    WorkspaceScrollDown,
    WorkspaceScrollUp,
    Surface(cosmic::surface::Action),
    Error(String),
    ToggleShowWorkspaceName(bool),
    RightClick(MenuTarget),
    MouseMove(cosmic::iced::Point),
    PinApp(String),
    UnpinApp(String),
    LaunchApp(String),
    Hover(Option<HoverTarget>),
    DragStart {
        app_id: String,
        index: usize,
    },
    DragOver {
        index: usize,
    },
    DragEnd,
    WindowDragStart {
        window_id: u64,
        workspace_id: u64,
    },
    WindowDragOver {
        target_workspace_id: u64,
        target: DragTarget,
    },
    WindowDragEnd,
}

/// Helper function to create custom button style for applet icons with specified alpha opacity.
fn icon_button_style(alpha: f32) -> cosmic::theme::Button {
    cosmic::theme::Button::Custom {
        active: Box::new(move |_focused, theme| {
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
        }),
        disabled: Box::new(move |theme| {
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
        }),
        hovered: Box::new(move |_focused, theme| {
            let cosmic = theme.cosmic();
            let mut style = cosmic::widget::button::Style::new();
            let h_alpha = (alpha + 0.10).clamp(0.20, 0.50);
            style.background = Some(cosmic::iced::Background::Color(cosmic::iced::Color {
                a: h_alpha,
                ..cosmic.accent_color().into()
            }));
            style.border_radius = cosmic.corner_radii.radius_s.into();
            style
        }),
        pressed: Box::new(move |_focused, theme| {
            let cosmic = theme.cosmic();
            let mut style = cosmic::widget::button::Style::new();
            let p_alpha = (alpha + 0.20).clamp(0.30, 0.60);
            style.background = Some(cosmic::iced::Background::Color(cosmic::iced::Color {
                a: p_alpha,
                ..cosmic.accent_color().into()
            }));
            style.border_radius = cosmic.corner_radii.radius_s.into();
            style
        }),
    }
}

/// Helper function to construct a vertical or horizontal divider with appropriate padding.
fn make_divider<'a>(is_horizontal: bool, pad_x: f32, pad_y: f32) -> Element<'a, Message> {
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

impl cosmic::Application for AppModel {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = "com.github.ton.CosmicExtNiriWindows";

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(
        core: cosmic::Core,
        _flags: Self::Flags,
    ) -> (Self, Task<cosmic::Action<Self::Message>>) {
        let (tx, mut rx) = cosmic::iced::futures::channel::mpsc::channel::<niri_ipc::Action>(100);

        let app = AppModel {
            core,
            raw_windows: HashMap::new(),
            raw_workspaces: HashMap::new(),
            display: Vec::new(),
            last_scroll_time: Instant::now(),
            action_tx: Some(tx),
            app_icon_cache: HashMap::new(),
            resolving_icons: std::collections::HashSet::new(),
            last_focused_window: None,
            show_workspace_name: false,
            context_menu_id: None,
            last_mouse_pos: cosmic::iced::Point::default(),
            pinned: load_pinned(),
            desktop_info_cache: HashMap::new(),
            hovered_target: None,
            context_menu_target: None,
            ignore_next_applet_right_click: false,
            dragged_item: None,
            has_drag_moved: false,
            drag_x_start: 0.0,
        };

        // Fetch initial list of windows and workspaces at startup
        let fetch_task = Task::perform(
            async {
                match Connection::make_connection().await {
                    Ok(mut conn) => {
                        let wins = match conn.push_request(Request::Windows).await {
                            Ok(Reply::Ok(Response::Windows(w))) => w,
                            _ => Vec::new(),
                        };
                        let wksps = match conn.push_request(Request::Workspaces).await {
                            Ok(Reply::Ok(Response::Workspaces(w))) => w,
                            _ => Vec::new(),
                        };
                        Ok((wins, wksps))
                    }
                    Err(e) => Err(e.to_string()),
                }
            },
            |result| match result {
                Ok((wins, wksps)) => Message::InitialData { wins, wksps },
                Err(e) => Message::Error(e),
            },
        );

        // Dedicated task to push actions to Niri without creating new connections per click
        let writer_task = Task::perform(
            async move {
                let mut conn_opt = None;
                while let Some(action) = cosmic::iced::futures::StreamExt::next(&mut rx).await {
                    if conn_opt.is_none() {
                        conn_opt = Connection::make_connection().await.ok();
                    }
                    if let Some(conn) = &mut conn_opt {
                        if conn
                            .push_request(Request::Action(action.clone()))
                            .await
                            .is_err()
                        {
                            // Retry once on failure
                            conn_opt = Connection::make_connection().await.ok();
                            if let Some(conn2) = &mut conn_opt {
                                let _ = conn2.push_request(Request::Action(action)).await;
                            }
                        }
                    }
                }
            },
            |_| Message::Error("IPC Writer Task Died".to_string()),
        );

        (
            app,
            Task::batch(vec![
                fetch_task.map(cosmic::Action::from),
                writer_task.map(cosmic::Action::from),
            ]),
        )
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let icon_size: u16 = match &self.core.applet.size {
            cosmic::applet::Size::Hardcoded((w, h)) => *w.min(h),
            cosmic::applet::Size::PanelSize(s) => s.get_applet_icon_size_with_padding(false) as u16,
        };

        let is_horizontal = self.core.applet.is_horizontal();
        let anchor = self.core.applet.anchor;

        let icon_f = (icon_size as f32) * 0.75;
        let render_icon_size = icon_f as u16;
        let btn_padding_x = (icon_f * 0.03).max(1.0);
        let btn_padding_y = ((icon_f * 0.03) / 2.0).max(0.5);
        let spacing = (icon_f * 0.05).max(1.0);
        let divider_y_padding = ((icon_f * 0.30) / 2.0).max(1.5);
        let divider_x_padding = (icon_f * 0.20).max(3.0);
        let wksp_label_size = icon_f * 0.40;
        let dot_width = (icon_f * 0.50).max(4.0);
        let dot_height = (icon_f * 0.10).max(2.0);
        let dot_radius = dot_height / 2.0;
        let dot_gap = (icon_f * 0.06).max(1.0);

        let dot_width_val = if is_horizontal { dot_width } else { dot_height };
        let dot_height_val = if is_horizontal { dot_height } else { dot_width };

        let mut children = Vec::new();

        let has_inactive_pinned = self.pinned.iter().any(|app_id| {
            !self
                .raw_windows
                .values()
                .any(|w| w.app_id.as_ref().map_or(false, |id| id == app_id))
        });

        if (self.display.is_empty() || self.display.iter().all(|ws| ws.windows.is_empty()))
            && !has_inactive_pinned
        {
            children.push(
                cosmic::iced::widget::Space::new()
                    .width(1.0)
                    .height(1.0)
                    .into(),
            );
        } else {
            let mut is_first_ws = true;

            // Render start divider
            let start_pad_x = if self.show_workspace_name {
                0.0
            } else {
                divider_x_padding
            };
            children.push(make_divider(is_horizontal, start_pad_x, divider_y_padding));

            // Render Pinned Icons (closed apps only)
            let mut has_pinned = false;
            for (idx, app_id) in self.pinned.iter().enumerate() {
                let is_open = self
                    .raw_windows
                    .values()
                    .any(|w| w.app_id.as_ref().map_or(false, |id| id == app_id));

                if !is_open {
                    has_pinned = true;

                    let icon_handle = self.get_icon_handle(Some(app_id), render_icon_size);

                    // Zero-allocation hover check
                    let is_hovered = matches!(&self.hovered_target, Some(HoverTarget::Pinned(id)) if id == app_id);
                    let opacity: f32 = if is_hovered { 1.0 } else { 0.5 };
                    let btn_alpha = if is_hovered { 0.15 } else { 0.0 };

                    let icon_element: Element<'_, Self::Message> = match icon_handle.data.clone() {
                        cosmic::widget::icon::Data::Image(image_handle) => {
                            cosmic::iced::widget::Image::new(image_handle)
                                .width(cosmic::iced::Length::Fixed(icon_f))
                                .height(cosmic::iced::Length::Fixed(icon_f))
                                .opacity(opacity)
                                .into()
                        }
                        cosmic::widget::icon::Data::Svg(svg_handle) => {
                            cosmic::iced::widget::Svg::<cosmic::Theme>::new(svg_handle)
                                .width(cosmic::iced::Length::Fixed(icon_f))
                                .height(cosmic::iced::Length::Fixed(icon_f))
                                .opacity(opacity)
                                .into()
                        }
                    };

                    let pinned_btn = widget::button::custom(icon_element)
                        .padding([btn_padding_y, btn_padding_x])
                        .class(icon_button_style(btn_alpha));

                    let app_id_clone1 = app_id.clone();
                    let app_id_clone2 = app_id.clone();
                    let track_area = cosmic::iced::widget::mouse_area(pinned_btn)
                        .on_enter(Message::DragOver { index: idx })
                        .on_exit(Message::Hover(None))
                        .on_press(Message::DragStart {
                            app_id: app_id_clone1,
                            index: idx,
                        })
                        .on_release(Message::DragEnd)
                        .on_right_release(Message::RightClick(MenuTarget::Pinned {
                            app_id: app_id_clone2,
                        }))
                        .interaction(cosmic::iced::mouse::Interaction::Pointer);

                    // Zero I/O friendly name lookup directly from memory cache
                    let friendly_name = self
                        .desktop_info_cache
                        .get(app_id)
                        .map(|info| info.name.clone())
                        .unwrap_or_else(|| app_id.clone());

                    let tooltip = self.core.applet.applet_tooltip(
                        track_area,
                        friendly_name,
                        false,
                        Message::Surface,
                        None,
                    );
                    children.push(tooltip.into());
                }
            }

            for workspace in &self.display {
                if workspace.windows.is_empty() {
                    continue;
                }

                if !is_first_ws || has_pinned || !self.show_workspace_name {
                    let mid_div = cosmic::iced::widget::mouse_area(make_divider(
                        is_horizontal,
                        divider_x_padding,
                        divider_y_padding,
                    ))
                    .on_enter(Message::WindowDragOver {
                        target_workspace_id: workspace.id,
                        target: DragTarget::WorkspaceStart,
                    });
                    children.push(mid_div.into());
                }
                is_first_ws = false;

                if self.show_workspace_name {
                    let label = workspace
                        .name
                        .clone()
                        .unwrap_or_else(|| workspace.idx.to_string());
                    let label_widget = cosmic::iced::widget::text(label).size(wksp_label_size);

                    let label_padding = if is_horizontal {
                        [0.0, divider_x_padding, 0.0, divider_x_padding]
                    } else {
                        [divider_x_padding, 0.0, divider_x_padding, 0.0]
                    };

                    let label_container = cosmic::iced::widget::mouse_area(
                        cosmic::widget::container(label_widget).padding(label_padding),
                    )
                    .on_enter(Message::WindowDragOver {
                        target_workspace_id: workspace.id,
                        target: DragTarget::WorkspaceStart,
                    });

                    children.push(label_container.into());
                }

                for window in &workspace.windows {
                    let icon_handle = self.get_icon_handle(window.app_id.as_deref(), render_icon_size);
                    let icon_widget = widget::icon(icon_handle).size(render_icon_size);

                    let dot = if window.is_focused {
                        cosmic::widget::container(
                            cosmic::iced::widget::Space::new()
                                .width(dot_width_val)
                                .height(dot_height_val),
                        )
                        .class(cosmic::theme::Container::custom(
                            move |t| cosmic::widget::container::Style {
                                background: Some(cosmic::iced::Background::Color(
                                    t.cosmic().accent_color().into(),
                                )),
                                border: cosmic::iced::Border {
                                    radius: dot_radius.into(),
                                    ..Default::default()
                                },
                                ..Default::default()
                            },
                        ))
                    } else {
                        cosmic::widget::container(
                            cosmic::iced::widget::Space::new()
                                .width(dot_width_val)
                                .height(dot_height_val),
                        )
                        .class(cosmic::theme::Container::custom(|_| {
                            cosmic::widget::container::Style {
                                background: None,
                                border: cosmic::iced::Border {
                                    color: cosmic::iced::Color::TRANSPARENT,
                                    width: 0.0,
                                    radius: 0.0.into(),
                                },
                                ..Default::default()
                            }
                        }))
                    };

                    let (content, padding): (Element<'_, Self::Message>, _) = match anchor {
                        cosmic::applet::cosmic_panel_config::PanelAnchor::Top => {
                            let content = cosmic::iced::widget::column![
                                dot,
                                cosmic::iced::widget::Space::new().height(dot_gap),
                                icon_widget,
                            ]
                            .align_x(cosmic::iced::Alignment::Center);
                            let padding = [
                                btn_padding_y,
                                btn_padding_x,
                                btn_padding_y + dot_gap + dot_height,
                                btn_padding_x,
                            ];
                            (content.into(), padding)
                        }
                        cosmic::applet::cosmic_panel_config::PanelAnchor::Left => {
                            let content = cosmic::iced::widget::row![
                                dot,
                                cosmic::iced::widget::Space::new().width(dot_gap),
                                icon_widget,
                            ]
                            .align_y(cosmic::iced::Alignment::Center);
                            let padding = [
                                btn_padding_y,
                                btn_padding_x + dot_gap + dot_height,
                                btn_padding_y,
                                btn_padding_x,
                            ];
                            (content.into(), padding)
                        }
                        cosmic::applet::cosmic_panel_config::PanelAnchor::Right => {
                            let content = cosmic::iced::widget::row![
                                icon_widget,
                                cosmic::iced::widget::Space::new().width(dot_gap),
                                dot,
                            ]
                            .align_y(cosmic::iced::Alignment::Center);
                            let padding = [
                                btn_padding_y,
                                btn_padding_x,
                                btn_padding_y,
                                btn_padding_x + dot_gap + dot_height,
                            ];
                            (content.into(), padding)
                        }
                        _ => {
                            let content = cosmic::iced::widget::column![
                                icon_widget,
                                cosmic::iced::widget::Space::new().height(dot_gap),
                                dot,
                            ]
                            .align_x(cosmic::iced::Alignment::Center);
                            let padding = [
                                btn_padding_y + dot_gap + dot_height,
                                btn_padding_x,
                                btn_padding_y,
                                btn_padding_x,
                            ];
                            (content.into(), padding)
                        }
                    };

                    let is_hovered = matches!(&self.hovered_target, Some(HoverTarget::Window(id)) if *id == window.id);
                    let btn_alpha = if window.is_focused && is_hovered {
                        0.30
                    } else if window.is_focused {
                        0.18
                    } else if is_hovered {
                        0.15
                    } else {
                        0.0
                    };

                    let btn = widget::button::custom(content)
                        .padding(padding)
                        .class(icon_button_style(btn_alpha));

                    let area = cosmic::iced::widget::mouse_area(btn)
                        .on_press(Message::WindowDragStart {
                            window_id: window.id,
                            workspace_id: workspace.id,
                        })
                        .on_enter(Message::WindowDragOver {
                            target_workspace_id: workspace.id,
                            target: DragTarget::Window(window.id),
                        })
                        .on_exit(Message::Hover(None))
                        .on_release(Message::WindowDragEnd)
                        .on_middle_press(Message::CloseWindow(window.id))
                        .on_right_release(Message::RightClick(MenuTarget::Window {
                            id: window.id,
                            app_id: window.app_id.clone(),
                        }));

                    let title = window.title.clone();
                    let tooltip =
                        self.core
                            .applet
                            .applet_tooltip(area, title, false, Message::Surface, None);

                    children.push(tooltip.into());
                }
            }

            if !is_first_ws || has_pinned {
                let end_ws_id = self.display.last().map(|w| w.id).unwrap_or(0);
                let last_div = cosmic::iced::widget::mouse_area(make_divider(
                    is_horizontal,
                    divider_x_padding,
                    divider_y_padding,
                ))
                .on_enter(Message::WindowDragOver {
                    target_workspace_id: end_ws_id,
                    target: DragTarget::WorkspaceEnd,
                });
                children.push(last_div.into());
            }
        }

        let applet_content: Element<'_, Self::Message> = if is_horizontal {
            Row::with_children(children)
                .spacing(spacing)
                .align_y(cosmic::iced::Alignment::Center)
                .width(cosmic::iced::Length::Shrink)
                .height(cosmic::iced::Length::Shrink)
                .into()
        } else {
            Column::with_children(children)
                .spacing(spacing)
                .align_x(cosmic::iced::Alignment::Center)
                .width(cosmic::iced::Length::Shrink)
                .height(cosmic::iced::Length::Shrink)
                .into()
        };

        let applet_area = cosmic::iced::widget::mouse_area(applet_content)
            .on_scroll(|delta| match delta {
                cosmic::iced::mouse::ScrollDelta::Lines { y, .. }
                | cosmic::iced::mouse::ScrollDelta::Pixels { y, .. } => {
                    if y < 0.0 {
                        Message::WorkspaceScrollDown
                    } else {
                        Message::WorkspaceScrollUp
                    }
                }
            })
            .on_right_release(Message::RightClick(MenuTarget::Applet))
            .on_move(|point| Message::MouseMove(point));

        self.core.applet.autosize_window(applet_area).into()
    }

    fn view_window(&self, _id: Id) -> Element<'_, Self::Message> {
        cosmic::iced::widget::Space::new().into()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        let niri_sub = Subscription::run(|| {
            cosmic::iced::stream::channel(
                100,
                move |mut channel: cosmic::iced::futures::channel::mpsc::Sender<Message>| async move {
                    loop {
                        if let Ok(conn) = Connection::make_connection().await {
                            if let Ok(mut listener) = conn.to_listener().await {
                                let mut buf = String::new();
                                while let Ok(Some(event)) = listener.next_event(&mut buf).await {
                                    let _ = channel.try_send(Message::NiriEvent(event));
                                    buf.clear();
                                }
                            }
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                },
            )
        });

        let mouse_sub = cosmic::iced::event::listen_with(|event, _status, _window_id| {
            if let cosmic::iced::Event::Mouse(cosmic::iced::mouse::Event::ButtonReleased(
                cosmic::iced::mouse::Button::Left,
            )) = event
            {
                Some(Message::DragEnd)
            } else {
                None
            }
        });

        Subscription::batch(vec![niri_sub, mouse_sub])
    }

    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        match message {
            Message::InitialData { wins, wksps } => {
                let our_output = self.core.applet.output_name.clone();
                self.raw_workspaces = wksps
                    .into_iter()
                    .filter(|ws| our_output.is_empty() || ws.output.as_deref() == Some(&our_output))
                    .map(|ws| (ws.id, ws))
                    .collect();

                self.raw_windows = wins
                    .into_iter()
                    .filter(|w| {
                        w.workspace_id
                            .map_or(false, |ws_id| self.raw_workspaces.contains_key(&ws_id))
                    })
                    .map(|w| (w.id, w))
                    .collect();

                if let Some(w) = self.raw_windows.values().find(|w| w.is_focused) {
                    self.last_focused_window = Some(w.id);
                } else if let Some(w) = self.raw_windows.values().next() {
                    self.last_focused_window = Some(w.id);
                }

                self.rebuild_display();
                self.pre_cache_pinned_desktop_paths();

                return self.spawn_missing_icon_tasks();
            }
            Message::IconResolved { app_id, handle } => {
                self.resolving_icons.remove(&app_id);
                self.app_icon_cache.insert(app_id, handle);
            }
            Message::NiriEvent(event) => {
                let task = match event {
                    Event::WorkspacesChanged { workspaces } => {
                        let our_output = self.core.applet.output_name.clone();
                        self.raw_workspaces = workspaces
                            .into_iter()
                            .filter(|ws| {
                                our_output.is_empty() || ws.output.as_deref() == Some(&our_output)
                            })
                            .map(|ws| (ws.id, ws))
                            .collect();

                        self.raw_windows.retain(|_, w| {
                            w.workspace_id
                                .map_or(false, |ws_id| self.raw_workspaces.contains_key(&ws_id))
                        });

                        self.rebuild_display();
                        Task::none()
                    }
                    Event::WorkspaceActivated { id, focused } => {
                        if let Some(ws) = self.raw_workspaces.get_mut(&id) {
                            ws.is_active = true;
                            ws.is_focused = focused;
                            for other in self.raw_workspaces.values_mut() {
                                if other.id != id {
                                    other.is_active = false;
                                    other.is_focused = false;
                                }
                            }
                            self.rebuild_display();
                        }
                        Task::none()
                    }
                    Event::WindowsChanged { windows } => {
                        self.raw_windows = windows
                            .into_iter()
                            .filter(|w| {
                                w.workspace_id
                                    .map_or(false, |ws_id| self.raw_workspaces.contains_key(&ws_id))
                            })
                            .map(|w| (w.id, w))
                            .collect();

                        self.update_last_focused_window();
                        self.rebuild_display();

                        self.spawn_missing_icon_tasks()
                    }
                    Event::WindowOpenedOrChanged { window } => {
                        let id = window.id;
                        let is_focused = window.is_focused;

                        if window
                            .workspace_id
                            .map_or(false, |ws_id| self.raw_workspaces.contains_key(&ws_id))
                        {
                            self.raw_windows.insert(id, window);
                            if is_focused {
                                self.last_focused_window = Some(id);
                            }
                            self.rebuild_display();
                            self.spawn_missing_icon_tasks()
                        } else {
                            if self.raw_windows.remove(&id).is_some() {
                                if self.last_focused_window == Some(id) {
                                    self.update_last_focused_window();
                                }
                                self.rebuild_display();
                            }
                            Task::none()
                        }
                    }
                    Event::WindowClosed { id } => {
                        if self.raw_windows.remove(&id).is_some() {
                            if self.last_focused_window == Some(id) {
                                self.update_last_focused_window();
                            }
                            self.rebuild_display();
                        }
                        Task::none()
                    }
                    Event::WindowFocusChanged { id } => {
                        if let Some(focused_id) = id {
                            if self.raw_windows.contains_key(&focused_id) {
                                self.last_focused_window = Some(focused_id);
                                self.rebuild_display();
                            }
                        }
                        Task::none()
                    }
                    Event::WindowUrgencyChanged { id, urgent } => {
                        if let Some(w) = self.raw_windows.get_mut(&id) {
                            w.is_urgent = urgent;
                            self.rebuild_display();
                        }
                        Task::none()
                    }
                    Event::WindowLayoutsChanged { changes, .. } => {
                        let mut changed = false;
                        for (id, new_layout) in changes {
                            if let Some(w) = self.raw_windows.get_mut(&id) {
                                w.layout = new_layout;
                                changed = true;
                            }
                        }
                        if changed {
                            self.rebuild_display();
                        }
                        Task::none()
                    }
                    _ => Task::none(),
                };
                return task;
            }
            Message::FocusWindow(window_id) => {
                if self.last_focused_window == Some(window_id) {
                    self.send_action(Action::CenterWindow {
                        id: Some(window_id),
                    });
                } else {
                    self.last_focused_window = Some(window_id);
                    self.rebuild_display();
                    self.send_action(Action::FocusWindow { id: window_id });
                }
            }
            Message::CloseWindow(window_id) => {
                self.send_action(Action::CloseWindow {
                    id: Some(window_id),
                });
            }
            Message::WorkspaceScrollDown => {
                if self.last_scroll_time.elapsed() >= std::time::Duration::from_millis(200) {
                    self.last_scroll_time = Instant::now();
                    self.send_action(Action::FocusWorkspaceDown {});
                }
            }
            Message::WorkspaceScrollUp => {
                if self.last_scroll_time.elapsed() >= std::time::Duration::from_millis(200) {
                    self.last_scroll_time = Instant::now();
                    self.send_action(Action::FocusWorkspaceUp {});
                }
            }
            Message::Surface(action) => {
                return Task::done(cosmic::Action::Cosmic(cosmic::app::Action::Surface(action)));
            }
            Message::PopupClosed(id) => {
                if Some(id) == self.context_menu_id {
                    self.context_menu_id = None;
                }
            }
            Message::Error(err) => {
                eprintln!("COSMIC Niri Applet Error: {}", err);
            }
            Message::ToggleShowWorkspaceName(val) => {
                self.show_workspace_name = val;
                self.rebuild_display();
                if let Some(task) = self.close_popup() {
                    return task;
                }
            }
            Message::RightClick(target) => {
                match &target {
                    MenuTarget::Applet => {
                        if self.ignore_next_applet_right_click {
                            self.ignore_next_applet_right_click = false;
                            return Task::none();
                        }
                    }
                    _ => {
                        self.ignore_next_applet_right_click = true;
                    }
                }

                if let Some(task) = self.close_popup() {
                    return task;
                } else {
                    self.context_menu_target = Some(target);
                    return self.open_context_menu();
                }
            }
            Message::MouseMove(point) => {
                self.last_mouse_pos = point;
                if self.dragged_item.is_some() {
                    if (point.x - self.drag_x_start).abs() > 6.0 {
                        self.has_drag_moved = true;
                    }
                }
            }
            Message::DragStart { app_id, index } => {
                self.dragged_item = Some(DraggedItem::Pinned { app_id, index });
                self.drag_x_start = self.last_mouse_pos.x;
                self.has_drag_moved = false;
            }
            Message::DragOver { index } => {
                let dragged_info = match &self.dragged_item {
                    Some(DraggedItem::Pinned { app_id, index }) => Some((app_id.clone(), *index)),
                    _ => None,
                };

                if let Some((app_id, dragged_idx)) = dragged_info {
                    if dragged_idx != index && index < self.pinned.len() {
                        self.pinned.remove(dragged_idx);
                        self.pinned.insert(index, app_id.clone());
                        self.dragged_item = Some(DraggedItem::Pinned {
                            app_id: app_id.clone(),
                            index,
                        });
                        self.hovered_target = Some(HoverTarget::Pinned(app_id));
                        self.rebuild_display();
                    }
                } else if index < self.pinned.len() {
                    let hovered_id = self.pinned[index].clone();
                    self.hovered_target = Some(HoverTarget::Pinned(hovered_id));
                }
            }
            Message::DragEnd => {
                if let Some(item) = self.dragged_item.take() {
                    match item {
                        DraggedItem::Pinned { app_id, .. } => {
                            if !self.has_drag_moved {
                                self.has_drag_moved = false;
                                return self.update(Message::LaunchApp(app_id));
                            } else {
                                self.has_drag_moved = false;
                                return save_pinned_async(self.pinned.clone())
                                    .map(cosmic::Action::from);
                            }
                        }
                        DraggedItem::Window { window_id, .. } => {
                            if !self.has_drag_moved {
                                self.has_drag_moved = false;
                                return self.update(Message::FocusWindow(window_id));
                            }
                        }
                    }
                }
                self.has_drag_moved = false;
            }
            Message::WindowDragStart {
                window_id,
                workspace_id,
            } => {
                self.dragged_item = Some(DraggedItem::Window {
                    window_id,
                    workspace_id,
                });
                self.drag_x_start = self.last_mouse_pos.x;
                self.has_drag_moved = false;
                self.send_action(Action::FocusWindow { id: window_id });
            }
            Message::WindowDragOver {
                target_workspace_id,
                target,
            } => {
                self.hovered_target = match &target {
                    DragTarget::Window(id) => Some(HoverTarget::Window(*id)),
                    _ => None,
                };
                let dragged_window_info = match &self.dragged_item {
                    Some(DraggedItem::Window { window_id, workspace_id }) => Some((*window_id, *workspace_id)),
                    _ => None,
                };
                if let Some((window_id, current_ws_id)) = dragged_window_info {
                    if self.has_drag_moved {
                        if target_workspace_id != current_ws_id {
                            // Move window across workspaces and focus target workspace
                            self.send_action(Action::MoveWindowToWorkspace {
                                window_id: Some(window_id),
                                reference: niri_ipc::WorkspaceReferenceArg::Id(
                                    target_workspace_id,
                                ),
                                focus: true,
                            });
                            self.send_action(Action::FocusWorkspace {
                                reference: niri_ipc::WorkspaceReferenceArg::Id(
                                    target_workspace_id,
                                ),
                            });
                            self.send_action(Action::FocusWindow { id: window_id });

                            // Instantly synchronize local memory state and rebuild display UI
                            if let Some(w) = self.raw_windows.get_mut(&window_id) {
                                w.workspace_id = Some(target_workspace_id);
                                w.is_focused = true;
                            }
                            for ws in self.raw_workspaces.values_mut() {
                                if ws.id == target_workspace_id {
                                    ws.is_active = true;
                                    ws.is_focused = true;
                                } else {
                                    ws.is_active = false;
                                    ws.is_focused = false;
                                }
                            }
                            self.last_focused_window = Some(window_id);
                            self.dragged_item = Some(DraggedItem::Window {
                                window_id,
                                workspace_id: target_workspace_id,
                            });
                            self.rebuild_display();
                        } else {
                            match target {
                                DragTarget::WorkspaceStart => {
                                    self.send_action(Action::MoveColumnToFirst {});
                                }
                                DragTarget::WorkspaceEnd => {
                                    self.send_action(Action::MoveColumnToLast {});
                                }
                                DragTarget::Window(t_win_id) => {
                                    if t_win_id != window_id {
                                        if let Some(ws) =
                                            self.display.iter().find(|w| w.id == current_ws_id)
                                        {
                                            if let Some(target_pos) =
                                                ws.windows.iter().position(|w| w.id == t_win_id)
                                            {
                                                self.send_action(Action::MoveColumnToIndex {
                                                    index: target_pos + 1,
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Message::WindowDragEnd => {
                return self.update(Message::DragEnd);
            }
            Message::PinApp(app_id) => {
                if !self.pinned.contains(&app_id) {
                    self.pinned.push(app_id.clone());
                    let save_task = save_pinned_async(self.pinned.clone());
                    self.rebuild_display();

                    if let Some(path) = crate::utils::find_desktop_file_path(&app_id) {
                        let name = crate::utils::get_desktop_entry_name(&path)
                            .unwrap_or_else(|| app_id.clone());
                        self.desktop_info_cache
                            .insert(app_id.clone(), DesktopAppInfo { path, name });
                    }

                    let pre_cache_icons = self.spawn_missing_icon_tasks();
                    let mut tasks = vec![save_task.map(cosmic::Action::from), pre_cache_icons];

                    if let Some(task) = self.close_popup() {
                        tasks.push(task);
                    }

                    return Task::batch(tasks);
                }
            }
            Message::UnpinApp(app_id) => {
                if let Some(pos) = self.pinned.iter().position(|x| x == &app_id) {
                    self.pinned.remove(pos);
                    let save_task = save_pinned_async(self.pinned.clone());
                    self.rebuild_display();

                    let mut tasks = vec![save_task.map(cosmic::Action::from)];

                    if let Some(task) = self.close_popup() {
                        tasks.push(task);
                    }

                    return Task::batch(tasks);
                }
            }
            Message::LaunchApp(app_id) => {
                let path_opt = if let Some(info) = self.desktop_info_cache.get(&app_id) {
                    Some(info.path.clone())
                } else if let Some(path) = crate::utils::find_desktop_file_path(&app_id) {
                    let name = crate::utils::get_desktop_entry_name(&path)
                        .unwrap_or_else(|| app_id.clone());
                    self.desktop_info_cache.insert(
                        app_id.clone(),
                        DesktopAppInfo {
                            path: path.clone(),
                            name,
                        },
                    );
                    Some(path)
                } else {
                    None
                };

                let cmd = if let Some(path) = path_opt {
                    vec![
                        "gio".to_string(),
                        "launch".to_string(),
                        path.to_string_lossy().into_owned(),
                    ]
                } else {
                    vec![app_id]
                };
                self.send_action(Action::Spawn { command: cmd });
            }
            Message::Hover(target) => {
                self.hovered_target = target;
            }
        }
        Task::none()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

impl AppModel {
    /// Retrieves or generates a fallback icon handle for the specified app_id.
    fn get_icon_handle(&self, app_id_opt: Option<&str>, icon_size: u16) -> widget::icon::Handle {
        if let Some(app_id) = app_id_opt {
            if let Some(handle) = self.app_icon_cache.get(app_id) {
                return handle.clone();
            }
        }
        widget::icon::from_name("preferences-system-windows-symbolic")
            .symbolic(false)
            .size(icon_size)
            .into()
    }

    fn rebuild_display(&mut self) {
        let mut workspaces: Vec<&Workspace> = self.raw_workspaces.values().collect();
        workspaces.sort_by_key(|ws| ws.idx);

        let mut display = Vec::new();

        for ws in workspaces {
            let mut ws_windows: Vec<&Window> = self
                .raw_windows
                .values()
                .filter(|w| w.workspace_id == Some(ws.id))
                .filter(|w| w.app_id.as_deref() != Some(Self::APP_ID))
                .collect();

            ws_windows.sort_by_key(|w| {
                w.layout
                    .pos_in_scrolling_layout
                    .unwrap_or((usize::MAX, usize::MAX))
            });

            let window_views: Vec<WindowView> = ws_windows
                .into_iter()
                .map(|w| {
                    let is_focused = Some(w.id) == self.last_focused_window;
                    let title = w.title.clone().unwrap_or_else(|| "Window".to_string());
                    WindowView {
                        id: w.id,
                        is_focused,
                        title,
                        app_id: w.app_id.clone(),
                    }
                })
                .collect();

            display.push(WorkspaceView {
                id: ws.id,
                idx: ws.idx,
                name: ws.name.clone(),
                windows: window_views,
            });
        }

        self.display = display;
    }

    fn resolve_icon_async(&self, app_id_str: String) -> Task<Message> {
        Task::perform(
            async move {
                let icon_name = crate::utils::find_fallback_icon(&app_id_str)
                    .unwrap_or_else(|| app_id_str.clone());

                let icon_name_clean = if icon_name.starts_with("file://") {
                    icon_name.trim_start_matches("file://").to_string()
                } else {
                    icon_name
                };

                let handle: widget::icon::Handle = if icon_name_clean.starts_with('/')
                    || std::path::Path::new(&icon_name_clean).exists()
                {
                    widget::icon::from_path(std::path::PathBuf::from(icon_name_clean))
                } else {
                    widget::icon::from_name(icon_name_clean)
                        .symbolic(false)
                        .into()
                };

                (app_id_str, handle)
            },
            |(app_id, handle)| Message::IconResolved { app_id, handle },
        )
    }

    fn spawn_missing_icon_tasks(&mut self) -> Task<cosmic::Action<Message>> {
        let mut tasks = Vec::new();
        let mut ids_to_resolve = std::collections::HashSet::new();

        for w in self.raw_windows.values() {
            let app_id_str = w
                .app_id
                .as_deref()
                .unwrap_or("preferences-system-windows-symbolic")
                .to_string();
            ids_to_resolve.insert(app_id_str);
        }

        for app_id in &self.pinned {
            ids_to_resolve.insert(app_id.clone());
        }

        for app_id_str in ids_to_resolve {
            if !self.app_icon_cache.contains_key(&app_id_str)
                && !self.resolving_icons.contains(&app_id_str)
            {
                self.resolving_icons.insert(app_id_str.clone());
                tasks.push(
                    self.resolve_icon_async(app_id_str)
                        .map(cosmic::Action::from),
                );
            }
        }
        if tasks.is_empty() {
            Task::none()
        } else {
            Task::batch(tasks)
        }
    }

    fn open_context_menu(&mut self) -> Task<cosmic::Action<Message>> {
        let popup_id = Id::unique();
        self.context_menu_id = Some(popup_id);

        let action = cosmic::surface::action::app_popup(
            |_app| cosmic::surface::action::LiveSettings::default(),
            move |app| {
                let parent = app.core.main_window_id().unwrap_or(Id::RESERVED);
                let mut settings = app
                    .core
                    .applet
                    .get_popup_settings(parent, popup_id, None, None, None);

                settings.positioner.anchor_rect = cosmic::iced::Rectangle {
                    x: app.last_mouse_pos.x as i32,
                    y: app.last_mouse_pos.y as i32,
                    width: 1,
                    height: 1,
                };

                settings
            },
            Some(Box::new(move |app: &AppModel| {
                let mut content_col = Column::new().spacing(8);

                let name_toggler = widget::toggler(app.show_workspace_name)
                    .label(Some("Show workspace names".to_string()))
                    .width(cosmic::iced::Length::Fill)
                    .spacing(20.0)
                    .on_toggle(Message::ToggleShowWorkspaceName);

                match &app.context_menu_target {
                    Some(MenuTarget::Window { id, app_id }) => {
                        // 1. Close window
                        let close_btn = widget::button::custom(widget::text("Close Window"))
                            .on_press(Message::CloseWindow(*id))
                            .class(cosmic::theme::Button::Text)
                            .width(cosmic::iced::Length::Fill);
                        content_col = content_col.push(close_btn);

                        // 2. Pin window (Pin Application)
                        if let Some(app_id) = app_id {
                            let is_pinned = app.pinned.contains(app_id);
                            let app_id_clone = app_id.clone();
                            let pin_action = if is_pinned {
                                Message::UnpinApp(app_id_clone)
                            } else {
                                Message::PinApp(app_id_clone)
                            };

                            let pin_toggler = widget::toggler(is_pinned)
                                .label(Some("Pin Application".to_string()))
                                .width(cosmic::iced::Length::Fill)
                                .spacing(20.0)
                                .on_toggle(move |_| pin_action.clone());

                            content_col = content_col.push(pin_toggler);
                        }

                        content_col =
                            content_col.push(cosmic::widget::divider::horizontal::default());
                    }
                    Some(MenuTarget::Pinned { app_id }) => {
                        let app_id_clone = app_id.clone();
                        let pin_toggler = widget::toggler(true)
                            .label(Some("Pin Application".to_string()))
                            .width(cosmic::iced::Length::Fill)
                            .spacing(20.0)
                            .on_toggle(move |_| Message::UnpinApp(app_id_clone.clone()));

                        content_col = content_col.push(pin_toggler);
                        content_col =
                            content_col.push(cosmic::widget::divider::horizontal::default());
                    }
                    _ => {}
                }

                // 3. Show workspace names
                content_col = content_col.push(name_toggler);

                let padded = cosmic::widget::container(content_col).padding(16);
                let popup_content = app.core.applet.popup_container(padded);

                Element::from(popup_content).map(cosmic::Action::App)
            })),
        );

        Task::done(cosmic::Action::Cosmic(cosmic::app::Action::Surface(action)))
    }

    fn send_action(&mut self, action: Action) {
        if let Some(tx) = &mut self.action_tx {
            let _ = tx.try_send(action);
        }
    }

    fn update_last_focused_window(&mut self) {
        self.last_focused_window = self
            .raw_windows
            .values()
            .find(|w| w.is_focused)
            .or_else(|| self.raw_windows.values().next())
            .map(|w| w.id);
    }

    fn close_popup(&mut self) -> Option<Task<cosmic::Action<Message>>> {
        self.context_menu_id.take().map(|popup_id| {
            self.context_menu_target = None;
            Task::done(cosmic::Action::Cosmic(cosmic::app::Action::Surface(
                cosmic::surface::action::destroy_popup(popup_id),
            )))
        })
    }

    fn pre_cache_pinned_desktop_paths(&mut self) {
        for app_id in &self.pinned {
            if !self.desktop_info_cache.contains_key(app_id) {
                if let Some(path) = crate::utils::find_desktop_file_path(app_id) {
                    let name = crate::utils::get_desktop_entry_name(&path)
                        .unwrap_or_else(|| app_id.clone());
                    self.desktop_info_cache
                        .insert(app_id.clone(), DesktopAppInfo { path, name });
                }
            }
        }
    }
}
