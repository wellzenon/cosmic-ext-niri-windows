use crate::niri::{Connection, Event};
use cosmic::iced::widget::Row;
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
}

#[derive(Debug, Clone)]
pub struct WorkspaceView {
    #[allow(dead_code)]
    pub id: u64,
    #[allow(dead_code)]
    pub idx: u8,
    pub windows: Vec<WindowView>,
}

pub struct AppModel {
    core: cosmic::Core,
    raw_windows: HashMap<u64, Window>,
    raw_workspaces: HashMap<u64, Workspace>,
    display: Vec<WorkspaceView>,
    last_scroll_time: Instant,
    action_tx: Option<cosmic::iced::futures::channel::mpsc::Sender<niri_ipc::Action>>,
    icon_cache: HashMap<u64, widget::icon::Handle>,
    last_focused_window: Option<u64>,
}

#[derive(Debug, Clone)]
pub enum Message {
    PopupClosed(#[allow(dead_code)] Id),
    NiriEvent(Event),
    InitialData {
        wins: Vec<Window>,
        wksps: Vec<Workspace>,
    },
    FocusWindow(u64),
    CloseWindow(u64),
    WorkspaceScrollDown,
    WorkspaceScrollUp,
    Surface(cosmic::surface::Action),
    Error(String),
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
            icon_cache: HashMap::new(),
            last_focused_window: None,
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

        let init_cmds = Task::batch(vec![
            fetch_task.map(cosmic::Action::from),
            writer_task.map(cosmic::Action::from),
        ]);

        (app, init_cmds)
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let suggested_size = self.core.applet.suggested_size(false);

        let mut icon_size = suggested_size.0.max(suggested_size.1);
        if icon_size == 0 {
            icon_size = match std::env::var("COSMIC_PANEL_SIZE").as_deref() {
                Ok("XS") => 16,
                Ok("S") => 20,
                Ok("M") => 24,
                Ok("L") => 32,
                Ok("XL") => 48,
                _ => 24,
            };
        }

        let icon_f = icon_size as f32;
        let btn_padding = (icon_f * 0.15).max(1.0);
        let btn_top_padding = btn_padding + (dot_height + dot_gap) / 2.0;
        let row_spacing = (icon_f * 0.20).max(1.0);
        let divider_block_padding = (icon_f * 0.30).max(3.0);
        let dot_width = (icon_f * 0.50).max(4.0);
        let dot_height = (icon_f * 0.10).max(2.0);
        let dot_radius = dot_height / 2.0;
        let dot_gap = (icon_f * 0.06).max(1.0);

        let mut row = Row::new()
            .spacing(row_spacing)
            .align_y(cosmic::iced::Alignment::Center);

        if self.display.is_empty() || self.display.iter().all(|ws| ws.windows.is_empty()) {
            // Render a minimal 1px transparent space to satisfy Wayland geometry requirements
            // without showing any placeholder icon.
            row = row.push(cosmic::iced::widget::Space::new().width(1.0).height(1.0));
        } else {
            let mut is_first_ws = true;
            for workspace in &self.display {
                if workspace.windows.is_empty() {
                    continue;
                }
                if !is_first_ws {
                    row = row.push(
                        cosmic::widget::container(cosmic::widget::divider::vertical::default())
                            .padding([divider_block_padding as u16, btn_padding as u16]),
                    );
                }
                is_first_ws = false;

                for window in &workspace.windows {
                    // Grab icon handle from cache, or use fallback if not found yet (should be resolved by update)
                    let icon_handle =
                        self.icon_cache.get(&window.id).cloned().unwrap_or_else(|| {
                            widget::icon::from_name("preferences-system-windows-symbolic")
                                .symbolic(false)
                                .size(icon_size)
                                .into()
                        });

                    let icon_widget = widget::icon(icon_handle).size(icon_size);

                    let dot = if window.is_focused {
                        cosmic::widget::container(
                            cosmic::iced::widget::Space::new()
                                .width(dot_width)
                                .height(dot_height),
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
                                .width(dot_width)
                                .height(dot_height),
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

                    let content = cosmic::iced::widget::column![
                        icon_widget,
                        cosmic::iced::widget::Space::new().height(dot_gap),
                        dot,
                    ]
                    .align_x(cosmic::iced::Alignment::Center);

                    let padded_content = cosmic::widget::container(content).padding([
                        btn_top_padding,
                        btn_padding,
                        btn_padding,
                        btn_padding,
                    ]);

                    let area = cosmic::iced::widget::mouse_area(padded_content)
                        .on_press(Message::FocusWindow(window.id))
                        .on_middle_press(Message::CloseWindow(window.id));

                    let title = window.title.clone();
                    let tooltip =
                        self.core
                            .applet
                            .applet_tooltip(area, title, false, Message::Surface, None);

                    row = row.push(tooltip);
                }
            }
        }

        let row = row
            .width(cosmic::iced::Length::Shrink)
            .height(cosmic::iced::Length::Shrink);

        let applet_area = cosmic::iced::widget::mouse_area(row).on_scroll(|delta| match delta {
            cosmic::iced::mouse::ScrollDelta::Lines { y, .. }
            | cosmic::iced::mouse::ScrollDelta::Pixels { y, .. } => {
                if y < 0.0 {
                    Message::WorkspaceScrollDown
                } else {
                    Message::WorkspaceScrollUp
                }
            }
        });

        self.core.applet.autosize_window(applet_area).into()
    }

    fn view_window(&self, _id: Id) -> Element<'_, Self::Message> {
        cosmic::iced::widget::Space::new().into()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        Subscription::run(|| {
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
        })
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

                // Resolve icons
                let icons_to_resolve: Vec<(u64, Option<String>)> = self
                    .raw_windows
                    .values()
                    .map(|w| (w.id, w.app_id.clone()))
                    .collect();
                for (id, app_id) in icons_to_resolve {
                    self.resolve_icon(id, app_id.as_deref());
                }

                // Find initial focus on our output
                if let Some(w) = self.raw_windows.values().find(|w| w.is_focused) {
                    self.last_focused_window = Some(w.id);
                } else if let Some(w) = self.raw_windows.values().next() {
                    self.last_focused_window = Some(w.id);
                }

                self.rebuild_display();
            }
            Message::NiriEvent(event) => match event {
                Event::WorkspacesChanged { workspaces } => {
                    let our_output = self.core.applet.output_name.clone();
                    self.raw_workspaces = workspaces
                        .into_iter()
                        .filter(|ws| {
                            our_output.is_empty() || ws.output.as_deref() == Some(&our_output)
                        })
                        .map(|ws| (ws.id, ws))
                        .collect();

                    // Retain only windows belonging to active workspaces on our output
                    self.raw_windows.retain(|_, w| {
                        w.workspace_id
                            .map_or(false, |ws_id| self.raw_workspaces.contains_key(&ws_id))
                    });

                    self.rebuild_display();
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

                    self.icon_cache
                        .retain(|id, _| self.raw_windows.contains_key(id));

                    let icons_to_resolve: Vec<(u64, Option<String>)> = self
                        .raw_windows
                        .values()
                        .map(|w| (w.id, w.app_id.clone()))
                        .collect();
                    for (id, app_id) in icons_to_resolve {
                        self.resolve_icon(id, app_id.as_deref());
                    }

                    if let Some(w) = self.raw_windows.values().find(|w| w.is_focused) {
                        self.last_focused_window = Some(w.id);
                    }
                    self.rebuild_display();
                }
                Event::WindowOpenedOrChanged { window } => {
                    let id = window.id;
                    let is_focused = window.is_focused;

                    if window
                        .workspace_id
                        .map_or(false, |ws_id| self.raw_workspaces.contains_key(&ws_id))
                    {
                        self.resolve_icon(id, window.app_id.as_deref());
                        self.raw_windows.insert(id, window);
                        if is_focused {
                            self.last_focused_window = Some(id);
                        }
                        self.rebuild_display();
                    } else {
                        // If it moved to another output, remove it from our cache
                        if self.raw_windows.remove(&id).is_some() {
                            self.icon_cache.remove(&id);
                            if self.last_focused_window == Some(id) {
                                self.last_focused_window = None;
                                if let Some(w) = self.raw_windows.values().find(|w| w.is_focused) {
                                    self.last_focused_window = Some(w.id);
                                } else if let Some(w) = self.raw_windows.values().next() {
                                    self.last_focused_window = Some(w.id);
                                }
                            }
                            self.rebuild_display();
                        }
                    }
                }
                Event::WindowClosed { id } => {
                    if self.raw_windows.remove(&id).is_some() {
                        self.icon_cache.remove(&id);
                        if self.last_focused_window == Some(id) {
                            self.last_focused_window = None;
                            if let Some(w) = self.raw_windows.values().find(|w| w.is_focused) {
                                self.last_focused_window = Some(w.id);
                            } else if let Some(w) = self.raw_windows.values().next() {
                                self.last_focused_window = Some(w.id);
                            }
                        }
                        self.rebuild_display();
                    }
                }
                Event::WindowFocusChanged { id } => {
                    if let Some(focused_id) = id {
                        if self.raw_windows.contains_key(&focused_id) {
                            self.last_focused_window = Some(focused_id);
                            self.rebuild_display();
                        }
                    }
                }
                Event::WindowUrgencyChanged { id, urgent } => {
                    if let Some(w) = self.raw_windows.get_mut(&id) {
                        w.is_urgent = urgent;
                        self.rebuild_display();
                    }
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
                }
                _ => {}
            },
            Message::FocusWindow(window_id) => {
                self.last_focused_window = Some(window_id);
                self.rebuild_display();
                if let Some(tx) = &mut self.action_tx {
                    let _ = tx.try_send(Action::FocusWindow { id: window_id });
                }
            }
            Message::CloseWindow(window_id) => {
                if let Some(tx) = &mut self.action_tx {
                    let _ = tx.try_send(Action::CloseWindow {
                        id: Some(window_id),
                    });
                }
            }
            Message::WorkspaceScrollDown => {
                if self.last_scroll_time.elapsed() >= std::time::Duration::from_millis(200) {
                    self.last_scroll_time = Instant::now();
                    if let Some(tx) = &mut self.action_tx {
                        let _ = tx.try_send(Action::FocusWorkspaceDown {});
                    }
                }
            }
            Message::WorkspaceScrollUp => {
                if self.last_scroll_time.elapsed() >= std::time::Duration::from_millis(200) {
                    self.last_scroll_time = Instant::now();
                    if let Some(tx) = &mut self.action_tx {
                        let _ = tx.try_send(Action::FocusWorkspaceUp {});
                    }
                }
            }
            Message::Surface(action) => {
                return Task::done(cosmic::Action::Cosmic(cosmic::app::Action::Surface(action)));
            }
            Message::PopupClosed(_) => {}
            Message::Error(err) => {
                eprintln!("COSMIC Niri Applet Error: {}", err);
            }
        }
        Task::none()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

impl AppModel {
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
                    }
                })
                .collect();

            display.push(WorkspaceView {
                id: ws.id,
                idx: ws.idx,
                windows: window_views,
            });
        }

        self.display = display;
    }

    fn resolve_icon(&mut self, id: u64, app_id: Option<&str>) {
        if self.icon_cache.contains_key(&id) {
            return;
        }
        let app_id_str = app_id.unwrap_or("preferences-system-windows-symbolic");

        let icon_name =
            crate::utils::find_fallback_icon(app_id_str).unwrap_or_else(|| app_id_str.to_string());

        let handle: widget::icon::Handle =
            widget::icon::from_name(icon_name).symbolic(false).into();

        self.icon_cache.insert(id, handle);
    }
}
