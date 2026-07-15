use crate::niri::{Connection, Event};
use cosmic::iced::widget::Row;
use cosmic::iced::{window::Id, Subscription, Task};
use cosmic::prelude::*;
use cosmic::widget;
use niri_ipc::{Action, Reply, Request, Response, Window, Workspace};
use std::collections::HashMap;
use std::time::Instant;

pub struct AppModel {
    core: cosmic::Core,
    windows: Vec<Window>,
    workspaces: Vec<Workspace>,
    last_scroll_time: Instant,
    action_tx: Option<cosmic::iced::futures::channel::mpsc::Sender<niri_ipc::Action>>,
    icon_cache: HashMap<u64, widget::icon::Handle>,
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
            windows: Vec::new(),
            workspaces: Vec::new(),
            last_scroll_time: Instant::now(),
            action_tx: Some(tx),
            icon_cache: HashMap::new(),
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
        let row_spacing = (icon_f * 0.20).max(1.0);
        let divider_block_padding = (icon_f * 0.30).max(3.0);
        let dot_width = (icon_f * 0.50).max(4.0);
        let dot_height = (icon_f * 0.10).max(2.0);
        let dot_radius = dot_height / 2.0;
        let dot_gap = (icon_f * 0.06).max(1.0);

        let mut row = Row::new()
            .spacing(row_spacing)
            .align_y(cosmic::iced::Alignment::Center);

        let sorted = self.sorted_windows();
        let current_output = &self.core.applet.output_name;

        let display_wins: Vec<&Window> = sorted
            .iter()
            .filter(|w| w.app_id.as_deref() != Some(Self::APP_ID))
            .filter(|w| {
                if !current_output.is_empty() {
                    if let Some(ws_id) = w.workspace_id {
                        if let Some(ws) = self.workspaces.iter().find(|ws| ws.id == ws_id) {
                            return ws.output.as_ref() == Some(current_output);
                        }
                    }
                    false
                } else {
                    true // If we don't know the panel output, show all
                }
            })
            .collect();

        if display_wins.is_empty() {
            // Render a minimal 1px transparent space to satisfy Wayland geometry requirements
            // without showing any placeholder icon.
            row = row.push(cosmic::iced::widget::Space::new().width(1.0).height(1.0));
        } else {
            let mut prev_workspace_id = None;
            let mut is_first = true;

            for window in display_wins {
                if !is_first {
                    if let (Some(prev_ws), Some(curr_ws)) = (prev_workspace_id, window.workspace_id)
                    {
                        if prev_ws != curr_ws {
                            row = row.push(
                                cosmic::widget::container(
                                    cosmic::widget::divider::vertical::default(),
                                )
                                .padding([divider_block_padding as u16, btn_padding as u16]),
                            );
                        }
                    }
                }
                prev_workspace_id = window.workspace_id;
                is_first = false;

                // Grab icon handle from cache, or use fallback if not found yet (should be resolved by update)
                let icon_handle = self.icon_cache.get(&window.id).cloned().unwrap_or_else(|| {
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
                    .class(cosmic::theme::Container::custom(move |t| {
                        cosmic::widget::container::Style {
                            background: Some(cosmic::iced::Background::Color(
                                t.cosmic().accent_color().into(),
                            )),
                            border: cosmic::iced::Border {
                                radius: dot_radius.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }
                    }))
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

                let top_padding = btn_padding + (dot_height + dot_gap) / 2.0;
                let padded_content = cosmic::widget::container(content).padding([
                    top_padding,
                    btn_padding,
                    btn_padding,
                    btn_padding,
                ]);

                let area = cosmic::iced::widget::mouse_area(padded_content)
                    .on_press(Message::FocusWindow(window.id))
                    .on_middle_press(Message::CloseWindow(window.id));

                let title = window.title.clone().unwrap_or_else(|| "Window".to_string());
                let tooltip =
                    self.core
                        .applet
                        .applet_tooltip(area, title, false, Message::Surface, None);

                row = row.push(tooltip);
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
                self.windows = wins;
                self.workspaces = wksps;
                let wins_clone = self.windows.clone();
                for w in wins_clone {
                    self.resolve_icon(&w);
                }
            }
            Message::NiriEvent(event) => match event {
                Event::WorkspacesChanged { workspaces } => {
                    self.workspaces = workspaces;
                }
                Event::WorkspaceActivated { id, focused } => {
                    if let Some(target) = self.workspaces.iter().find(|w| w.id == id).cloned() {
                        let target_output = target.output;
                        for ws in &mut self.workspaces {
                            if ws.output == target_output {
                                ws.is_active = ws.id == id;
                                ws.is_focused = ws.id == id && focused;
                            }
                        }
                    }
                }
                Event::WindowsChanged { windows } => {
                    self.windows = windows.clone();
                    for w in windows {
                        self.resolve_icon(&w);
                    }
                }
                Event::WindowOpenedOrChanged { window } => {
                    if window.is_focused {
                        for w in &mut self.windows {
                            w.is_focused = false;
                        }
                    }
                    self.resolve_icon(&window);

                    if let Some(idx) = self.windows.iter().position(|w| w.id == window.id) {
                        self.windows[idx] = window;
                    } else {
                        self.windows.push(window);
                    }
                }
                Event::WindowClosed { id } => {
                    self.windows.retain(|w| w.id != id);
                    self.icon_cache.remove(&id);
                }
                Event::WindowFocusChanged { id } => {
                    for w in &mut self.windows {
                        w.is_focused = id == Some(w.id);
                    }
                }
                Event::WindowUrgencyChanged { id, urgent } => {
                    if let Some(w) = self.windows.iter_mut().find(|w| w.id == id) {
                        w.is_urgent = urgent;
                    }
                }
                Event::WindowLayoutsChanged { changes, .. } => {
                    for (id, new_layout) in changes {
                        if let Some(w) = self.windows.iter_mut().find(|w| w.id == id) {
                            w.layout = new_layout;
                        }
                    }
                }
                _ => {}
            },
            Message::FocusWindow(window_id) => {
                for w in &mut self.windows {
                    w.is_focused = w.id == window_id;
                }
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
    fn resolve_icon(&mut self, window: &Window) {
        if self.icon_cache.contains_key(&window.id) {
            return;
        }
        let app_id = window
            .app_id
            .clone()
            .unwrap_or_else(|| "preferences-system-windows-symbolic".to_string());

        let icon_name = crate::utils::find_fallback_icon(&app_id).unwrap_or(app_id);

        // Cache the parsed Handle rather than recreating it on every frame
        let handle: widget::icon::Handle =
            widget::icon::from_name(icon_name).symbolic(false).into();

        self.icon_cache.insert(window.id, handle);
    }

    fn sorted_windows(&self) -> Vec<Window> {
        let mut sorted = self.windows.clone();
        sorted.sort_by(|a, b| {
            let ws_a = a
                .workspace_id
                .and_then(|id| self.workspaces.iter().find(|w| w.id == id));
            let ws_b = b
                .workspace_id
                .and_then(|id| self.workspaces.iter().find(|w| w.id == id));

            let idx_a = ws_a.map(|w| w.idx).unwrap_or(u8::MAX);
            let idx_b = ws_b.map(|w| w.idx).unwrap_or(u8::MAX);

            match idx_a.cmp(&idx_b) {
                std::cmp::Ordering::Equal => {
                    let pos_a = a
                        .layout
                        .pos_in_scrolling_layout
                        .unwrap_or((usize::MAX, usize::MAX));
                    let pos_b = b
                        .layout
                        .pos_in_scrolling_layout
                        .unwrap_or((usize::MAX, usize::MAX));
                    pos_a.cmp(&pos_b)
                }
                other => other,
            }
        });
        sorted
    }
}
