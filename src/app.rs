use crate::config::AppConfig;
use crate::niri::{Connection, Event};
use crate::session::{OutputSnapshot, SessionSnapshot, WindowSnapshot, WorkspaceSnapshot};
use cosmic::iced::futures::SinkExt;
use cosmic::iced::widget::{Column, Row};
use cosmic::iced::{Subscription, Task, window::Id};
use cosmic::prelude::*;
use cosmic::widget;
#[cfg(target_os = "linux")]
use iced_winit::platform_specific::commands::layer_surface::{
    KeyboardInteractivity, set_keyboard_interactivity,
};
use niri_ipc::{Action, Output, Reply, Request, Response, Window, Workspace};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::time::Instant;

#[derive(Debug, Clone)]
pub struct DesktopAppInfo {
    pub path: std::path::PathBuf,
    pub name: String,
}

fn save_config_task(config: AppConfig) -> Task<Message> {
    Task::perform(
        async move { crate::config::save(&config).await },
        Message::ConfigSaveFinished,
    )
}

fn save_session_task(
    path: Option<std::path::PathBuf>,
    snapshot: SessionSnapshot,
    write_lock: Arc<tokio::sync::Mutex<()>>,
    generation: u64,
    latest_generation: Arc<AtomicU64>,
) -> Task<Message> {
    Task::perform(
        async move {
            match path {
                Some(path) => {
                    if latest_generation.load(Ordering::Acquire) != generation {
                        return Ok(());
                    }
                    let _guard = write_lock.lock().await;
                    if latest_generation.load(Ordering::Acquire) != generation {
                        return Ok(());
                    }
                    crate::session::save_to(&path, snapshot).await
                }
                None => Err("session path is unavailable".to_string()),
            }
        },
        Message::SessionSaveFinished,
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

#[derive(Default)]
pub struct DragState {
    pub item: Option<DraggedItem>,
    pub start: cosmic::iced::Point,
    pub has_moved: bool,
    pub focused_before: bool,
}

#[derive(Default)]
pub struct WidthSliderState {
    pub dragging: HashMap<u64, u32>,
}

#[derive(Default)]
pub struct ContextMenuState {
    pub popup_id: Option<Id>,
    pub target: Option<MenuTarget>,
    pub snapshot_age: Option<String>,
}

pub struct IconCache {
    pub handles: HashMap<String, widget::icon::Handle>,
    pub resolving: HashSet<String>,
    resolver_limit: Arc<tokio::sync::Semaphore>,
}

impl Default for IconCache {
    fn default() -> Self {
        Self {
            handles: HashMap::new(),
            resolving: HashSet::new(),
            // Icon themes can contain thousands of files. Avoid monopolizing the
            // runtime when many windows appear at once.
            resolver_limit: Arc::new(tokio::sync::Semaphore::new(2)),
        }
    }
}

pub struct AppModel {
    // ── Core ──────────────────────────────────────────────
    core: cosmic::Core,
    launcher: crate::launcher::SystemLauncher,
    action_tx: Option<cosmic::iced::futures::channel::mpsc::Sender<niri_ipc::Action>>,

    // ── Niri Data ─────────────────────────────────────────
    raw_windows: HashMap<u64, Window>,
    raw_workspaces: HashMap<u64, Workspace>,
    raw_outputs: HashMap<String, Output>,
    ordered_workspace_ids: Vec<u64>,
    windows_by_workspace: HashMap<u64, Vec<u64>>,
    open_app_ids: HashSet<String>,
    column_presets: Vec<crate::utils::ColumnPreset>,
    last_focused_window: Option<u64>,

    // ── Pinned Apps ───────────────────────────────────────
    config: AppConfig,
    pinned: Vec<String>,
    desktop_info_cache: HashMap<String, DesktopAppInfo>,
    canonical_app_ids: HashMap<String, String>,

    // ── Grouped State ─────────────────────────────────────
    icons: IconCache,
    menu: ContextMenuState,
    drag: DragState,
    width_slider: WidthSliderState,

    // ── UI ────────────────────────────────────────────────
    last_mouse_pos: cosmic::iced::Point,
    last_scroll_time: Instant,
    hovered_target: Option<HoverTarget>,
    show_workspace_name: bool,
    ignore_next_applet_right_click: bool,

    // ── Session Persistence ──────────────────────────────
    session_enabled: bool,
    session_generation: u64,
    session_write_lock: Arc<tokio::sync::Mutex<()>>,
    latest_session_generation: Arc<AtomicU64>,
    restoring_session: bool,
    pending_restore: Vec<(WorkspaceSnapshot, WindowSnapshot)>,
    restore_temporary_workspaces: Vec<String>,
    restore_snapshot: Option<SessionSnapshot>,
}

#[derive(Debug, Clone)]
pub enum Message {
    NiriEvent(Event),
    InitialData {
        wins: Vec<Window>,
        wksps: Vec<Workspace>,
        outs: HashMap<String, Output>,
    },
    IconResolved {
        app_id: String,
        handle: widget::icon::Handle,
    },
    CloseWindow(u64),
    SetColumnWidth(u64, String),
    ColumnWidthSliderChanged(u64, u32),
    ColumnWidthSliderReleased(u64),
    WorkspaceScrollDown,
    WorkspaceScrollUp,
    Surface(cosmic::surface::Action),
    Error(String),
    OutputsUpdated(HashMap<String, Output>),
    ToggleShowWorkspaceName(bool),
    ToggleSessionPersistence(bool),
    ToggleFixedSnapshot(bool),
    LaunchPendingSessionWindows,
    FinishSessionRestore,
    SessionSnapshotReady {
        generation: u64,
        snapshot: SessionSnapshot,
    },
    SessionSaveFinished(Result<(), String>),
    ConfigSaveFinished(Result<(), String>),
    RightClick(MenuTarget),
    MouseMove(cosmic::iced::Point),
    PinApp(String),
    UnpinApp(String),

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
    ClosePopupAndDrag,
}

/// Helper function to create the background style for button states.
fn create_button_style(theme: &cosmic::Theme, alpha: f32) -> cosmic::widget::button::Style {
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

/// Helper function to create custom button style for applet icons with specified alpha opacity.
fn icon_button_style(alpha: f32) -> cosmic::theme::Button {
    cosmic::theme::Button::Custom {
        active: Box::new(move |_focused, theme| create_button_style(theme, alpha)),
        disabled: Box::new(move |theme| create_button_style(theme, alpha)),
        hovered: Box::new(move |_focused, theme| {
            let h_alpha = (alpha + 0.10).clamp(0.20, 0.50);
            create_button_style(theme, h_alpha)
        }),
        pressed: Box::new(move |_focused, theme| {
            let p_alpha = (alpha + 0.20).clamp(0.30, 0.60);
            create_button_style(theme, p_alpha)
        }),
    }
}

/// Helper function to construct a vertical or horizontal divider with appropriate padding.
fn make_divider(is_horizontal: bool, pad_x: f32, pad_y: f32) -> Element<'static, Message> {
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

fn restore_app_ids_match(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    left.split_once("webapp-")
        .zip(right.split_once("webapp-"))
        .is_some_and(|((_, left_id), (_, right_id))| left_id == right_id)
}

fn scaled_restore_dimension(value: f64, saved_extent: Option<u32>, current_extent: u32) -> f64 {
    let Some(saved_extent) = saved_extent.filter(|extent| *extent > 0) else {
        return value;
    };
    (value * f64::from(current_extent) / f64::from(saved_extent))
        .round()
        .clamp(1.0, f64::from(current_extent))
}

impl cosmic::Application for AppModel {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = "io.github.wellzenon.CosmicExtNiriWindows";

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
        let config = match crate::config::load() {
            Ok(config) => config,
            Err(error) => {
                log::warn!("Could not load applet config: {error}");
                AppConfig::default()
            }
        };
        let loaded_session = crate::session::session_path()
            .and_then(|path| crate::session::load_from(&path).ok().flatten());
        let session_enabled = config.session_enabled;
        let show_workspace_names = config.show_workspace_names;
        let write_initial_config = crate::config::config_path()
            .map(|path| !path.exists())
            .unwrap_or(false);

        let app = AppModel {
            core,
            raw_windows: HashMap::new(),
            raw_workspaces: HashMap::new(),
            last_scroll_time: Instant::now(),
            action_tx: Some(tx),
            last_focused_window: None,
            show_workspace_name: show_workspace_names,
            last_mouse_pos: cosmic::iced::Point::default(),
            pinned: config.pinned.clone(),
            desktop_info_cache: HashMap::new(),
            canonical_app_ids: HashMap::new(),
            hovered_target: None,
            ignore_next_applet_right_click: false,
            column_presets: crate::utils::load_niri_column_presets(),
            raw_outputs: HashMap::new(),
            ordered_workspace_ids: Vec::new(),
            windows_by_workspace: HashMap::new(),
            open_app_ids: HashSet::new(),
            launcher: crate::launcher::SystemLauncher::detect(),
            icons: IconCache::default(),
            menu: ContextMenuState::default(),
            drag: DragState::default(),
            width_slider: WidthSliderState::default(),
            session_enabled,
            session_generation: 0,
            session_write_lock: Arc::new(tokio::sync::Mutex::new(())),
            latest_session_generation: Arc::new(AtomicU64::new(0)),
            restoring_session: false,
            pending_restore: Vec::new(),
            restore_temporary_workspaces: Vec::new(),
            restore_snapshot: loaded_session.filter(|_| session_enabled),
            config,
        };

        // Fetch initial list of windows, workspaces, and outputs at startup
        let fetch_task = Task::perform(
            async {
                match Connection::make_connection().await {
                    Ok(mut conn) => {
                        let wins = match conn.push_request(Request::Windows).await {
                            Ok(Reply::Ok(Response::Windows(w))) => w,
                            other => {
                                log::warn!("Failed to fetch windows: {:?}", other);
                                Vec::new()
                            }
                        };
                        let wksps = match conn.push_request(Request::Workspaces).await {
                            Ok(Reply::Ok(Response::Workspaces(w))) => w,
                            other => {
                                log::warn!("Failed to fetch workspaces: {:?}", other);
                                Vec::new()
                            }
                        };
                        let outs = match conn.push_request(Request::Outputs).await {
                            Ok(Reply::Ok(Response::Outputs(o))) => o,
                            other => {
                                log::warn!("Failed to fetch outputs: {:?}", other);
                                HashMap::new()
                            }
                        };
                        Ok((wins, wksps, outs))
                    }
                    Err(e) => Err(e.to_string()),
                }
            },
            |result| match result {
                Ok((wins, wksps, outs)) => Message::InitialData { wins, wksps, outs },
                Err(e) => Message::Error(e),
            },
        );

        // Dedicated task to push actions to Niri without creating new connections per click
        let writer_task = Task::perform(
            async move {
                let mut conn_opt = None;
                while let Some(action) = cosmic::iced::futures::StreamExt::next(&mut rx).await {
                    if conn_opt.is_none() {
                        match Connection::make_connection().await {
                            Ok(conn) => conn_opt = Some(conn),
                            Err(e) => log::warn!("IPC writer connection failed: {}", e),
                        }
                    }
                    if let Some(conn) = &mut conn_opt {
                        log::debug!("Sending action to Niri: {:?}", action);
                        if conn
                            .push_request(Request::Action(action.clone()))
                            .await
                            .is_err()
                        {
                            log::warn!("IPC action failed, reconnecting...");
                            match Connection::make_connection().await {
                                Ok(conn) => conn_opt = Some(conn),
                                Err(e) => {
                                    log::error!("IPC reconnection failed: {}", e);
                                    conn_opt = None;
                                }
                            }
                            if let Some(conn2) = &mut conn_opt
                                && let Err(e) = conn2.push_request(Request::Action(action)).await
                            {
                                log::error!("IPC action retry failed: {}", e);
                            }
                        }
                    }
                }
            },
            |_| Message::Error("IPC Writer Task Died".to_string()),
        );

        let main_window_id = cosmic::iced::window::Id::RESERVED;
        let no_keyboard_task =
            set_keyboard_interactivity::<Message>(main_window_id, KeyboardInteractivity::None)
                .map(cosmic::Action::from);
        let initial_config_task = if write_initial_config {
            save_config_task(app.config.clone()).map(cosmic::Action::from)
        } else {
            Task::none()
        };

        (
            app,
            Task::batch(vec![
                fetch_task.map(cosmic::Action::from),
                writer_task.map(cosmic::Action::from),
                no_keyboard_task,
                initial_config_task,
            ]),
        )
    }

    fn view(&self) -> Element<'_, Self::Message> {
        if self.restoring_session {
            return self
                .core
                .applet
                .autosize_window(cosmic::widget::text("Restoring session…").size(14))
                .into();
        }
        let has_active_windows = !self.windows_by_workspace.is_empty();

        let icon_size: u16 = match &self.core.applet.size {
            cosmic::applet::Size::Hardcoded((w, h)) => *w.min(h),
            cosmic::applet::Size::PanelSize(s) => s.get_applet_icon_size_with_padding(false) as u16,
        };

        let is_horizontal = self.core.applet.is_horizontal();
        let anchor = self.core.applet.anchor;

        let icon_f = (icon_size as f32) * 0.75;
        let render_icon_size = icon_f as u16;
        let btn_padding = (icon_f * 0.1).max(2.0);
        let btn_padding_x = btn_padding;
        let btn_padding_y = btn_padding;
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

        let dot_inactive_len = dot_height * 3.0; // Shorter pill for inactive
        let dot_inactive_width_val = if is_horizontal {
            dot_inactive_len
        } else {
            dot_height
        };
        let dot_inactive_height_val = if is_horizontal {
            dot_height
        } else {
            dot_inactive_len
        };

        let estimated_children = self.pinned.len()
            + self.raw_windows.len()
            + self.raw_workspaces.len().saturating_mul(2)
            + 2;
        let mut children = Vec::with_capacity(estimated_children);

        let has_inactive_pinned = self
            .pinned
            .iter()
            .any(|app_id| !self.open_app_ids.contains(app_id));

        if !has_active_windows && !has_inactive_pinned {
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
                let is_open = self.open_app_ids.contains(app_id);

                if !is_open {
                    has_pinned = true;

                    let icon_handle = self.get_icon_handle(Some(app_id), render_icon_size);

                    // Zero-allocation hover check
                    let is_hovered = matches!(&self.hovered_target, Some(HoverTarget::Pinned(id)) if id == app_id);
                    let opacity: f32 = if is_hovered { 1.0 } else { 0.5 };
                    let btn_alpha = 0.0;

                    let icon_element: Element<'_, Self::Message> = match &icon_handle.data {
                        cosmic::widget::icon::Data::Image(image_handle) => {
                            cosmic::iced::widget::Image::new(image_handle.clone())
                                .width(cosmic::iced::Length::Fixed(icon_f))
                                .height(cosmic::iced::Length::Fixed(icon_f))
                                .opacity(opacity)
                                .into()
                        }
                        cosmic::widget::icon::Data::Svg(svg_handle) => {
                            cosmic::iced::widget::Svg::<cosmic::Theme>::new(svg_handle.clone())
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

            for workspace_id in &self.ordered_workspace_ids {
                let Some(workspace) = self.raw_workspaces.get(workspace_id) else {
                    continue;
                };
                let Some(window_ids) = self.windows_by_workspace.get(&workspace.id) else {
                    continue;
                };

                if !is_first_ws || has_pinned {
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
                    let label_widget = if let Some(name) = &workspace.name {
                        cosmic::iced::widget::text(name)
                    } else {
                        cosmic::iced::widget::text(workspace.idx.to_string())
                    }
                    .size(wksp_label_size);

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

                for window_id in window_ids {
                    let Some(window) = self.raw_windows.get(window_id) else {
                        continue;
                    };
                    let is_focused = Some(window.id) == self.last_focused_window;
                    let icon_handle =
                        self.get_icon_handle(window.app_id.as_deref(), render_icon_size);
                    let icon_widget = widget::icon(icon_handle).size(render_icon_size);

                    let dot = if is_focused {
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
                                .width(dot_inactive_width_val)
                                .height(dot_inactive_height_val),
                        )
                        .class(cosmic::theme::Container::custom(
                            move |t| {
                                let mut color = t.cosmic().accent_color();
                                color.alpha = 0.3; // 30% opacity for inactive windows
                                cosmic::widget::container::Style {
                                    background: Some(cosmic::iced::Background::Color(color.into())),
                                    border: cosmic::iced::Border {
                                        radius: dot_radius.into(),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                }
                            },
                        ))
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

                    let btn_alpha = if is_focused { 0.18 } else { 0.0 };

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
                        .on_release(Message::DragEnd)
                        .on_middle_press(Message::CloseWindow(window.id))
                        .on_right_release(Message::RightClick(MenuTarget::Window {
                            id: window.id,
                            app_id: window.app_id.clone(),
                        }));

                    let title = window.title.clone().unwrap_or_else(|| "Window".to_string());
                    let tooltip =
                        self.core
                            .applet
                            .applet_tooltip(area, title, false, Message::Surface, None);

                    children.push(tooltip.into());
                }
            }

            if !is_first_ws || has_pinned {
                let end_ws_id = self.ordered_workspace_ids.last().copied().unwrap_or(0);
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
            .on_move(Message::MouseMove);

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
                        match Connection::make_connection().await {
                            Ok(conn) => match conn.into_listener().await {
                                Ok(mut listener) => {
                                    log::debug!("Niri event stream connected");
                                    let mut buf = String::new();
                                    while let Ok(maybe_event) = listener.next_event(&mut buf).await
                                    {
                                        if let Some(event) = maybe_event
                                            && channel
                                                .send(Message::NiriEvent(event))
                                                .await
                                                .is_err()
                                        {
                                            return;
                                        }
                                        buf.clear();
                                    }
                                    log::warn!("Niri event stream disconnected, reconnecting...");
                                }
                                Err(e) => log::warn!("Failed to start event listener: {}", e),
                            },
                            Err(e) => log::warn!("Event stream connection failed: {}", e),
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                },
            )
        });

        let global_sub =
            cosmic::iced::event::listen_with(|event, _status, _window_id| match event {
                cosmic::iced::Event::Mouse(cosmic::iced::mouse::Event::ButtonReleased(
                    cosmic::iced::mouse::Button::Left,
                )) => Some(Message::DragEnd),
                cosmic::iced::Event::Keyboard(cosmic::iced::keyboard::Event::KeyPressed {
                    ..
                }) => Some(Message::ClosePopupAndDrag),
                _ => None,
            });

        let subs = vec![niri_sub, global_sub];

        Subscription::batch(subs)
    }

    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        match message {
            Message::InitialData { wins, wksps, outs } => {
                self.raw_outputs = outs;
                let our_output = &self.core.applet.output_name;
                self.raw_workspaces = wksps
                    .into_iter()
                    .filter(|ws| our_output.is_empty() || ws.output.as_deref() == Some(our_output))
                    .map(|ws| (ws.id, ws))
                    .collect();
                self.rebuild_workspace_order();

                let mut windows: HashMap<u64, Window> = wins
                    .into_iter()
                    .filter(|w| {
                        w.workspace_id
                            .is_some_and(|ws_id| self.raw_workspaces.contains_key(&ws_id))
                    })
                    .map(|w| (w.id, w))
                    .collect();
                for window in windows.values_mut() {
                    self.normalize_window_app_id(window);
                }
                self.raw_windows = windows;
                self.rebuild_window_index();

                if let Some(w) = self.raw_windows.values().find(|w| w.is_focused) {
                    self.last_focused_window = Some(w.id);
                } else if let Some(w) = self.raw_windows.values().next() {
                    self.last_focused_window = Some(w.id);
                }

                self.pre_cache_pinned_desktop_paths();
                let icon_task = self.spawn_missing_icon_tasks();
                let restore_task = self.begin_session_restore();
                return Task::batch(vec![icon_task, restore_task]);
            }
            Message::IconResolved { app_id, handle } => {
                self.icons.resolving.remove(&app_id);
                self.icons.handles.insert(app_id, handle);
            }
            Message::NiriEvent(event) => return self.handle_niri_event(event),
            Message::CloseWindow(window_id) => {
                self.send_action(Action::CloseWindow {
                    id: Some(window_id),
                });
            }
            Message::SetColumnWidth(window_id, change_spec) => {
                self.handle_set_column_width(window_id, change_spec);
            }
            Message::ColumnWidthSliderChanged(window_id, val) => {
                self.handle_column_width_slider_changed(window_id, val);
            }
            Message::ColumnWidthSliderReleased(window_id) => {
                self.handle_column_width_slider_released(window_id);
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
            Message::Error(err) => {
                log::error!("{}", err);
            }
            Message::OutputsUpdated(outs) => {
                self.raw_outputs = outs;
            }
            Message::ToggleShowWorkspaceName(val) => {
                self.show_workspace_name = val;
                self.config.show_workspace_names = val;
                let config_task = save_config_task(self.config.clone()).map(cosmic::Action::from);
                if let Some(task) = self.close_popup() {
                    return Task::batch(vec![task, config_task]);
                }
                return config_task;
            }
            Message::ToggleSessionPersistence(enabled) => {
                self.session_enabled = enabled;
                self.config.session_enabled = enabled;
                if !enabled {
                    self.config.fixed_snapshot_enabled = false;
                    self.config.fixed_snapshot_enabled_at = None;
                }
                if enabled {
                    self.restore_snapshot = None;
                    self.pending_restore.clear();
                    self.restoring_session = false;
                    let save_task = self.schedule_session_save();
                    let config_task =
                        save_config_task(self.config.clone()).map(cosmic::Action::from);
                    if let Some(task) = self.close_popup() {
                        return Task::batch(vec![task, save_task, config_task]);
                    }
                    return Task::batch(vec![save_task, config_task]);
                }
                let snapshot = self.capture_session();
                self.session_generation = self.session_generation.wrapping_add(1);
                let generation = self.session_generation;
                self.latest_session_generation
                    .store(generation, Ordering::Release);
                let save_task = save_session_task(
                    crate::session::session_path(),
                    snapshot,
                    Arc::clone(&self.session_write_lock),
                    generation,
                    Arc::clone(&self.latest_session_generation),
                )
                .map(cosmic::Action::from);
                let config_task = save_config_task(self.config.clone()).map(cosmic::Action::from);
                if let Some(task) = self.close_popup() {
                    return Task::batch(vec![task, save_task, config_task]);
                }
                return Task::batch(vec![save_task, config_task]);
            }
            Message::SessionSnapshotReady {
                generation,
                snapshot,
            } => {
                if self.session_enabled && generation == self.session_generation {
                    return save_session_task(
                        crate::session::session_path(),
                        snapshot,
                        Arc::clone(&self.session_write_lock),
                        generation,
                        Arc::clone(&self.latest_session_generation),
                    )
                    .map(cosmic::Action::from);
                }
            }
            Message::ToggleFixedSnapshot(enabled) => {
                if !self.session_enabled {
                    return Task::none();
                }
                self.config.fixed_snapshot_enabled = enabled;
                self.config.fixed_snapshot_enabled_at = enabled.then(|| {
                    jiff::Timestamp::now()
                        .strftime("%Y-%m-%dT%H:%M:%SZ")
                        .to_string()
                });
                self.session_generation = self.session_generation.wrapping_add(1);
                let generation = self.session_generation;
                self.latest_session_generation
                    .store(generation, Ordering::Release);
                let config_task = save_config_task(self.config.clone()).map(cosmic::Action::from);
                let snapshot_task = if enabled {
                    save_session_task(
                        crate::session::session_path(),
                        self.capture_session(),
                        Arc::clone(&self.session_write_lock),
                        generation,
                        Arc::clone(&self.latest_session_generation),
                    )
                    .map(cosmic::Action::from)
                } else {
                    self.schedule_session_save()
                };
                if let Some(task) = self.close_popup() {
                    return Task::batch(vec![task, snapshot_task, config_task]);
                }
                return Task::batch(vec![snapshot_task, config_task]);
            }
            Message::LaunchPendingSessionWindows => {
                let app_ids: Vec<String> = self
                    .pending_restore
                    .iter()
                    .map(|(_, snapshot)| snapshot.app_id.clone())
                    .collect();
                for app_id in app_ids {
                    self.handle_launch_app(&app_id);
                }
            }
            Message::FinishSessionRestore => {
                for name in std::mem::take(&mut self.restore_temporary_workspaces) {
                    self.send_action(Action::UnsetWorkspaceName {
                        reference: Some(niri_ipc::WorkspaceReferenceArg::Name(name)),
                    });
                }
                self.restoring_session = false;
            }
            Message::SessionSaveFinished(result) => {
                if let Err(error) = result {
                    log::error!("Could not persist Niri session: {error}");
                }
            }
            Message::ConfigSaveFinished(result) => {
                if let Err(error) = result {
                    log::error!("Could not persist applet config: {error}");
                }
            }
            Message::RightClick(target) => return self.handle_right_click(target),
            Message::MouseMove(point) => {
                self.last_mouse_pos = point;
                if self.drag.item.is_some()
                    && ((point.x - self.drag.start.x).abs() > 6.0
                        || (point.y - self.drag.start.y).abs() > 6.0)
                {
                    self.drag.has_moved = true;
                }
            }
            Message::DragStart { app_id, index } => {
                self.drag.item = Some(DraggedItem::Pinned { app_id, index });
                self.drag.start = self.last_mouse_pos;
                self.drag.has_moved = false;
            }
            Message::DragOver { index } => {
                self.handle_drag_over(index);
            }
            Message::DragEnd => {
                return self.handle_drag_end();
            }
            Message::WindowDragStart {
                window_id,
                workspace_id,
            } => {
                self.handle_window_drag_start(window_id, workspace_id);
            }
            Message::WindowDragOver {
                target_workspace_id,
                target,
            } => {
                self.handle_window_drag_over(target_workspace_id, target);
            }
            Message::PinApp(app_id) => return self.handle_pin_app(app_id),
            Message::UnpinApp(app_id) => return self.handle_unpin_app(app_id),

            Message::Hover(target) => {
                self.hovered_target = target;
            }
            Message::ClosePopupAndDrag => {
                let mut tasks = vec![];
                if let Some(task) = self.close_popup() {
                    tasks.push(task);
                }

                let window_ids: Vec<u64> = self.width_slider.dragging.keys().copied().collect();
                for window_id in window_ids {
                    self.handle_column_width_slider_released(window_id);
                }
                return Task::batch(tasks);
            }
        }
        Task::none()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

impl AppModel {
    fn handle_set_column_width(&mut self, window_id: u64, change_spec: String) {
        let proportion = change_spec.ends_with('%');
        let size_str = change_spec.trim_end_matches(|c: char| c.is_alphabetic() || c == '%');
        if let Ok(size) = size_str.parse::<f64>() {
            self.send_column_width_action(window_id, size, proportion);
        } else {
            log::warn!("Invalid width specifier: {}", change_spec);
        }
    }

    fn handle_column_width_slider_changed(&mut self, window_id: u64, val: u32) {
        let current_width = self
            .width_slider
            .dragging
            .get(&window_id)
            .copied()
            .unwrap_or_else(|| self.calculate_window_width_pct(window_id));

        if val.abs_diff(current_width) > 89 {
            return;
        }

        self.width_slider.dragging.insert(window_id, val);
        self.send_column_width_action(window_id, val as f64, true);
    }

    fn handle_column_width_slider_released(&mut self, window_id: u64) {
        if let Some(val) = self.width_slider.dragging.remove(&window_id) {
            self.send_column_width_action(window_id, val as f64, true);
        }
    }

    fn send_column_width_action(&mut self, window_id: u64, size: f64, proportion: bool) {
        self.send_action(Action::FocusWindow { id: window_id });
        if proportion {
            self.send_action(Action::SetColumnWidth {
                change: niri_ipc::SizeChange::SetProportion(size),
            });
        } else {
            self.send_action(Action::SetColumnWidth {
                change: niri_ipc::SizeChange::SetFixed(size as i32),
            });
        }
    }

    fn handle_drag_over(&mut self, index: usize) {
        let dragged_info = match &self.drag.item {
            Some(DraggedItem::Pinned { app_id, index }) => Some((app_id.clone(), *index)),
            _ => None,
        };

        if let Some((app_id, dragged_idx)) = dragged_info {
            if dragged_idx != index && index < self.pinned.len() {
                self.pinned.remove(dragged_idx);
                self.pinned.insert(index, app_id.clone());
                self.drag.item = Some(DraggedItem::Pinned {
                    app_id: app_id.clone(),
                    index,
                });
                self.hovered_target = Some(HoverTarget::Pinned(app_id));
            }
        } else if index < self.pinned.len() {
            let hovered_id = self.pinned[index].clone();
            self.hovered_target = Some(HoverTarget::Pinned(hovered_id));
        }
    }

    fn handle_drag_end(&mut self) -> Task<cosmic::Action<Message>> {
        if let Some(item) = self.drag.item.take() {
            match item {
                DraggedItem::Pinned { app_id, .. } => {
                    if !self.drag.has_moved {
                        self.drag.has_moved = false;
                        self.handle_launch_app(&app_id);
                        return Task::none();
                    } else {
                        self.drag.has_moved = false;
                        self.config.pinned = self.pinned.clone();
                        return save_config_task(self.config.clone()).map(cosmic::Action::from);
                    }
                }
                DraggedItem::Window {
                    window_id,
                    workspace_id,
                } => {
                    if !self.drag.has_moved {
                        self.drag.has_moved = false;
                        self.focus_or_center_window(
                            window_id,
                            workspace_id,
                            self.drag.focused_before,
                        );
                    }
                }
            }
        }
        self.drag.has_moved = false;
        Task::none()
    }

    fn handle_window_drag_start(&mut self, window_id: u64, workspace_id: u64) {
        self.drag.item = Some(DraggedItem::Window {
            window_id,
            workspace_id,
        });
        self.drag.start = self.last_mouse_pos;
        self.drag.has_moved = false;
        self.drag.focused_before = self.last_focused_window == Some(window_id);
        if !self.drag.focused_before {
            self.send_action(niri_ipc::Action::FocusWindow { id: window_id });
            self.last_focused_window = Some(window_id);
            if let Some(w) = self.raw_windows.get_mut(&window_id) {
                w.is_focused = true;
            }
        }
    }

    fn handle_niri_event(&mut self, event: Event) -> Task<cosmic::Action<Message>> {
        match event {
            Event::WorkspacesChanged { workspaces } => {
                let our_output = &self.core.applet.output_name;
                // Update raw_workspaces based on the complete list provided by Niri
                self.raw_workspaces.retain(|id, _| {
                    workspaces.iter().any(|w| {
                        w.id == *id
                            && (our_output.is_empty() || w.output.as_deref() == Some(our_output))
                    })
                });
                for ws in workspaces {
                    if our_output.is_empty() || ws.output.as_deref() == Some(our_output) {
                        if let Some(existing) = self.raw_workspaces.get_mut(&ws.id) {
                            existing.idx = ws.idx;
                            existing.name = ws.name.clone();
                            existing.output = ws.output.clone();
                            existing.active_window_id = ws.active_window_id;
                        } else {
                            self.raw_workspaces.insert(ws.id, ws.clone());
                        }
                    }
                }
                self.raw_windows.retain(|_, window| {
                    window
                        .workspace_id
                        .is_some_and(|id| self.raw_workspaces.contains_key(&id))
                });
                self.rebuild_workspace_order();
                self.rebuild_window_index();
                self.with_session_save(Task::none())
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
                }
                self.with_session_save(Task::none())
            }
            Event::WindowsChanged { windows } => {
                let mut raw_windows: HashMap<u64, Window> = windows
                    .into_iter()
                    .filter(|w| {
                        w.workspace_id
                            .is_some_and(|ws_id| self.raw_workspaces.contains_key(&ws_id))
                    })
                    .map(|w| (w.id, w))
                    .collect();
                for window in raw_windows.values_mut() {
                    self.normalize_window_app_id(window);
                }
                self.raw_windows = raw_windows;
                self.rebuild_window_index();

                self.update_last_focused_window();
                let icon_task = self.spawn_missing_icon_tasks();
                self.with_session_save(icon_task)
            }
            Event::WindowOpenedOrChanged { mut window } => {
                self.normalize_window_app_id(&mut window);
                let id = window.id;
                let is_focused = window.is_focused;
                let new_workspace_id = window.workspace_id;
                let new_app_id = window.app_id.clone();
                let old_window = self.raw_windows.insert(id, window);
                let old_workspace_id = old_window.as_ref().and_then(|w| w.workspace_id);
                let old_app_id = old_window.as_ref().and_then(|w| w.app_id.clone());

                if new_workspace_id.is_some_and(|ws_id| self.raw_workspaces.contains_key(&ws_id)) {
                    self.update_window_indexes_after_change(
                        old_workspace_id,
                        new_workspace_id,
                        old_app_id,
                        new_app_id,
                    );
                    if is_focused {
                        self.last_focused_window = Some(id);
                    }
                    if let Some(window) = self.raw_windows.get(&id).cloned() {
                        let restore_finished = self.restore_new_window(&window);
                        let restore_task = if restore_finished {
                            Self::session_restore_transition_task()
                        } else {
                            Task::none()
                        };
                        let icon_task = self.spawn_missing_icon_tasks();
                        return Task::batch(vec![
                            icon_task,
                            restore_task,
                            self.schedule_session_save(),
                        ]);
                    }
                    let icon_task = self.spawn_missing_icon_tasks();
                    self.with_session_save(icon_task)
                } else {
                    self.raw_windows.remove(&id);
                    if self.last_focused_window == Some(id) {
                        self.update_last_focused_window();
                    }
                    self.update_window_indexes_after_change(
                        old_workspace_id,
                        None,
                        old_app_id,
                        None,
                    );
                    self.with_session_save(Task::none())
                }
            }
            Event::WindowClosed { id } => {
                let old_window = self.raw_windows.remove(&id);
                if old_window.is_some() && self.last_focused_window == Some(id) {
                    self.update_last_focused_window();
                }
                if let Some(old_window) = old_window {
                    self.update_window_indexes_after_change(
                        old_window.workspace_id,
                        None,
                        old_window.app_id,
                        None,
                    );
                }
                self.with_session_save(Task::none())
            }
            Event::WindowFocusChanged { id } => {
                if let Some(focused_id) = id
                    && self.raw_windows.contains_key(&focused_id)
                {
                    self.last_focused_window = Some(focused_id);
                }
                self.with_session_save(Task::none())
            }
            Event::WorkspaceActiveWindowChanged {
                workspace_id,
                active_window_id,
            } => {
                if let Some(workspace) = self.raw_workspaces.get_mut(&workspace_id) {
                    workspace.active_window_id = active_window_id;
                }
                self.with_session_save(Task::none())
            }
            Event::WindowUrgencyChanged { id, urgent } => {
                if let Some(w) = self.raw_windows.get_mut(&id) {
                    w.is_urgent = urgent;
                }
                Task::none()
            }
            Event::WindowLayoutsChanged { changes, .. } => {
                for (id, new_layout) in changes {
                    if let Some(w) = self.raw_windows.get_mut(&id) {
                        w.layout = new_layout;
                    }
                }
                // Window order is derived from `pos_in_scrolling_layout`. Keep the
                // render index synchronized with live drag/reorder events.
                self.rebuild_window_index();
                self.with_session_save(Task::none())
            }
            Event::ConfigLoaded { .. } => fetch_outputs_async().map(cosmic::Action::from),
            _ => Task::none(),
        }
    }

    fn handle_launch_app(&mut self, app_id: &str) {
        let path_opt = if let Some(info) = self.desktop_info_cache.get(app_id) {
            Some(info.path.clone())
        } else if let Some(path) = crate::utils::find_desktop_file_path(app_id) {
            let name =
                crate::utils::get_desktop_entry_name(&path).unwrap_or_else(|| app_id.to_string());
            self.desktop_info_cache.insert(
                app_id.to_string(),
                DesktopAppInfo {
                    path: path.clone(),
                    name,
                },
            );
            Some(path)
        } else {
            None
        };

        let mut is_terminal = false;
        let mut exec_args = None;

        if let Some(ref path) = path_opt
            && let Ok(entry) =
                freedesktop_desktop_entry::DesktopEntry::from_path(path, None::<&[&str]>)
        {
            is_terminal = entry.terminal();
            exec_args = entry.parse_exec().ok();
        }

        let cmd = self.launcher.build_command_with_args(
            app_id,
            path_opt.as_deref(),
            is_terminal,
            exec_args.as_deref(),
        );

        self.send_action(Action::Spawn { command: cmd });
    }

    fn handle_pin_app(&mut self, raw_app_id: String) -> Task<cosmic::Action<Message>> {
        let canonical_id = self.canonical_app_id(&raw_app_id);
        if !self.pinned.contains(&canonical_id) {
            self.pinned.push(canonical_id.clone());
            self.config.pinned = self.pinned.clone();
            let save_task = save_config_task(self.config.clone());

            if let Some(path) = crate::utils::find_desktop_file_path(&canonical_id) {
                let name = crate::utils::get_desktop_entry_name(&path)
                    .unwrap_or_else(|| canonical_id.clone());
                self.desktop_info_cache
                    .insert(canonical_id, DesktopAppInfo { path, name });
            }

            let pre_cache_icons = self.spawn_missing_icon_tasks();
            let mut tasks = vec![save_task.map(cosmic::Action::from), pre_cache_icons];

            if let Some(task) = self.close_popup() {
                tasks.push(task);
            }

            Task::batch(tasks)
        } else {
            Task::none()
        }
    }

    fn handle_unpin_app(&mut self, raw_app_id: String) -> Task<cosmic::Action<Message>> {
        let canonical_id = self.canonical_app_id(&raw_app_id);
        if let Some(pos) = self
            .pinned
            .iter()
            .position(|x| x == &raw_app_id || x == &canonical_id)
        {
            self.pinned.remove(pos);
            self.config.pinned = self.pinned.clone();
            let save_task = save_config_task(self.config.clone());

            let mut tasks = vec![save_task.map(cosmic::Action::from)];

            if let Some(task) = self.close_popup() {
                tasks.push(task);
            }

            Task::batch(tasks)
        } else {
            Task::none()
        }
    }

    fn handle_window_drag_over(&mut self, target_workspace_id: u64, target: DragTarget) {
        self.hovered_target = match &target {
            DragTarget::Window(id) => Some(HoverTarget::Window(*id)),
            _ => None,
        };
        let dragged_window_info = match &self.drag.item {
            Some(DraggedItem::Window {
                window_id,
                workspace_id,
            }) => Some((*window_id, *workspace_id)),
            _ => None,
        };
        if let Some((window_id, current_ws_id)) = dragged_window_info
            && self.drag.has_moved
        {
            if target_workspace_id != current_ws_id {
                let target_idx = self
                    .raw_workspaces
                    .get(&target_workspace_id)
                    .map(|w| w.idx)
                    .unwrap_or(0);
                self.send_action(Action::MoveWindowToWorkspace {
                    window_id: Some(window_id),
                    reference: niri_ipc::WorkspaceReferenceArg::Index(target_idx),
                    focus: true,
                });
                self.send_action(Action::FocusWorkspace {
                    reference: niri_ipc::WorkspaceReferenceArg::Index(target_idx),
                });
                self.send_action(Action::FocusWindow { id: window_id });

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
                self.rebuild_workspace_window_index(current_ws_id);
                self.rebuild_workspace_window_index(target_workspace_id);
                self.drag.item = Some(DraggedItem::Window {
                    window_id,
                    workspace_id: target_workspace_id,
                });
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
                            let mut ws_windows: Vec<&niri_ipc::Window> = self
                                .raw_windows
                                .values()
                                .filter(|w| {
                                    w.workspace_id == Some(current_ws_id)
                                        && w.app_id.as_deref()
                                            != Some(<Self as cosmic::Application>::APP_ID)
                                })
                                .collect();
                            ws_windows.sort_unstable_by_key(|w| {
                                w.layout
                                    .pos_in_scrolling_layout
                                    .unwrap_or((usize::MAX, usize::MAX))
                            });
                            if let Some(target_pos) =
                                ws_windows.iter().position(|w| w.id == t_win_id)
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

    fn handle_right_click(&mut self, target: MenuTarget) -> Task<cosmic::Action<Message>> {
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
            task
        } else {
            self.menu.snapshot_age = if target == MenuTarget::Applet {
                self.config
                    .fixed_snapshot_enabled_at
                    .as_deref()
                    .map(|timestamp| {
                        crate::utils::format_relative_time(timestamp, jiff::Timestamp::now())
                    })
            } else {
                None
            };
            self.menu.target = Some(target);
            self.open_context_menu()
        }
    }

    /// Retrieves or generates a fallback icon handle for the specified app_id.
    fn get_icon_handle(&self, app_id_opt: Option<&str>, icon_size: u16) -> widget::icon::Handle {
        if let Some(app_id) = app_id_opt
            && let Some(handle) = self.icons.handles.get(app_id)
        {
            return handle.clone();
        }
        widget::icon::from_name("preferences-system-windows-symbolic")
            .symbolic(false)
            .size(icon_size)
            .into()
    }

    fn resolve_icon_async(&self, app_id_str: String) -> Task<Message> {
        let resolver_limit = Arc::clone(&self.icons.resolver_limit);
        Task::perform(
            async move {
                let _permit = resolver_limit
                    .acquire_owned()
                    .await
                    .expect("icon resolver semaphore is never closed");
                let lookup_id = app_id_str.clone();
                let handle = tokio::task::spawn_blocking(move || {
                    let icon_name = crate::utils::find_fallback_icon(&lookup_id)
                        .unwrap_or_else(|| lookup_id.clone());
                    let icon_name_clean = if icon_name.starts_with("file://") {
                        icon_name.trim_start_matches("file://").to_string()
                    } else {
                        icon_name
                    };

                    // Desktop-entry Icon values are either absolute paths or icon
                    // names. A name must not be interpreted relative to the applet's
                    // working directory (for example, Icon=obsidian and ~/obsidian).
                    if std::path::Path::new(&icon_name_clean).is_absolute() {
                        widget::icon::from_path(std::path::PathBuf::from(icon_name_clean))
                    } else if let Some(path_str) =
                        crate::utils::resolve_icon_name_to_path(&icon_name_clean)
                    {
                        widget::icon::from_path(std::path::PathBuf::from(path_str))
                    } else {
                        widget::icon::from_name(icon_name_clean)
                            .symbolic(false)
                            .into()
                    }
                })
                .await
                .ok()
                .unwrap_or_else(|| {
                    widget::icon::from_name(app_id_str.clone())
                        .symbolic(false)
                        .into()
                });

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
            if !self.icons.handles.contains_key(&app_id_str)
                && !self.icons.resolving.contains(&app_id_str)
            {
                self.icons.resolving.insert(app_id_str.clone());
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

    fn build_width_controls(&self, window_id: u64) -> Element<'_, Message> {
        let current_width = self
            .width_slider
            .dragging
            .get(&window_id)
            .copied()
            .unwrap_or_else(|| self.calculate_window_width_pct(window_id));

        let width_label =
            cosmic::iced::widget::text(format!("Column Width: {}%", current_width)).size(13);

        let slider = cosmic::iced::widget::slider(10..=100, current_width, move |val| {
            Message::ColumnWidthSliderChanged(window_id, val)
        })
        .on_release(Message::ColumnWidthSliderReleased(window_id))
        .step(5u32)
        .width(cosmic::iced::Length::Fill);

        let mut width_row = Row::new().spacing(4);
        let mut has_100_pct = false;

        for preset in &self.column_presets {
            if preset.label == "100%" {
                has_100_pct = true;
            }
            let change_spec = preset.change_spec.clone();
            let btn = widget::button::custom(cosmic::iced::widget::text(&preset.label).size(12))
                .on_press(Message::SetColumnWidth(window_id, change_spec))
                .padding([4, 8]);
            width_row = width_row.push(btn);
        }

        if !has_100_pct {
            let max_btn = widget::button::custom(cosmic::iced::widget::text("100%").size(12))
                .on_press(Message::SetColumnWidth(window_id, "100%".to_string()))
                .padding([4, 8]);
            width_row = width_row.push(max_btn);
        }

        Column::new()
            .spacing(8)
            .push(width_label)
            .push(slider)
            .push(width_row)
            .into()
    }

    fn build_window_menu(&self, window_id: u64, app_id: &Option<String>) -> Element<'_, Message> {
        let mut content_col = Column::new().spacing(8);

        // 1. Close window
        let close_btn = widget::button::custom(widget::text("Close Window"))
            .on_press(Message::CloseWindow(window_id))
            .class(cosmic::theme::Button::Text)
            .width(cosmic::iced::Length::Fill);
        content_col = content_col.push(close_btn);
        content_col = content_col.push(cosmic::widget::divider::horizontal::default());

        // 2. Column Width Controls & Interactive Slider
        content_col = content_col.push(self.build_width_controls(window_id));
        content_col = content_col.push(cosmic::widget::divider::horizontal::default());

        // 3. Pin window (Pin Application)
        if let Some(app_id_str) = app_id {
            let is_pinned = self.pinned.contains(app_id_str);
            let pin_action = if is_pinned {
                Message::UnpinApp(app_id_str.clone())
            } else {
                Message::PinApp(app_id_str.clone())
            };

            let pin_toggler = widget::toggler(is_pinned)
                .label(Some("Pin Application".to_string()))
                .width(cosmic::iced::Length::Fill)
                .spacing(20.0)
                .on_toggle(move |_| pin_action.clone());

            content_col = content_col.push(pin_toggler);
            content_col = content_col.push(cosmic::widget::divider::horizontal::default());
        }

        content_col.into()
    }

    fn build_pinned_menu(&self, app_id: &str) -> Element<'_, Message> {
        let mut content_col = Column::new().spacing(8);
        let app_id_clone = app_id.to_string();
        let pin_toggler = widget::toggler(true)
            .label(Some("Pin Application".to_string()))
            .width(cosmic::iced::Length::Fill)
            .spacing(20.0)
            .on_toggle(move |_| Message::UnpinApp(app_id_clone.clone()));

        content_col = content_col.push(pin_toggler);
        content_col = content_col.push(cosmic::widget::divider::horizontal::default());
        content_col.into()
    }

    fn build_applet_menu(&self) -> Element<'_, Message> {
        let mut content_col = Column::new().spacing(8);
        let target_window_id = self
            .last_focused_window
            .filter(|id| self.raw_windows.contains_key(id));

        if let Some(window_id) = target_window_id {
            content_col = content_col.push(self.build_width_controls(window_id));
            content_col = content_col.push(cosmic::widget::divider::horizontal::default());
        }

        let session_toggler = widget::toggler(self.session_enabled)
            .label(Some("Restore session on restart".to_string()))
            .width(cosmic::iced::Length::Fill)
            .spacing(20.0)
            .on_toggle(Message::ToggleSessionPersistence);
        content_col = content_col.push(session_toggler);
        let snapshot_toggler =
            widget::toggler(self.config.fixed_snapshot_enabled).width(cosmic::iced::Length::Shrink);
        let snapshot_toggler = if self.session_enabled {
            snapshot_toggler.on_toggle(Message::ToggleFixedSnapshot)
        } else {
            snapshot_toggler
        };
        let mut snapshot_row = Row::new()
            .spacing(8)
            .align_y(cosmic::iced::Alignment::Center)
            .width(cosmic::iced::Length::Fill)
            .push(cosmic::widget::text("Freeze session"));
        if let Some(age) = &self.menu.snapshot_age {
            let age_label = cosmic::widget::text(format!("({age})")).size(12).class(
                cosmic::theme::Text::Color(cosmic::iced::Color {
                    r: 0.55,
                    g: 0.55,
                    b: 0.55,
                    a: 0.75,
                }),
            );
            snapshot_row = snapshot_row.push(age_label);
        }
        snapshot_row = snapshot_row
            .push(cosmic::iced::widget::Space::new().width(cosmic::iced::Length::Fill))
            .push(snapshot_toggler);
        content_col = content_col.push(snapshot_row);
        content_col.into()
    }

    fn open_context_menu(&mut self) -> Task<cosmic::Action<Message>> {
        let popup_id = Id::unique();
        self.menu.popup_id = Some(popup_id);

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
                let target_content = match &app.menu.target {
                    Some(MenuTarget::Window { id, app_id }) => app.build_window_menu(*id, app_id),
                    Some(MenuTarget::Pinned { app_id }) => app.build_pinned_menu(app_id),
                    _ => app.build_applet_menu(),
                };

                let name_toggler = widget::toggler(app.show_workspace_name)
                    .label(Some("Show workspace names".to_string()))
                    .width(cosmic::iced::Length::Fill)
                    .spacing(20.0)
                    .on_toggle(Message::ToggleShowWorkspaceName);

                let content_col = Column::new()
                    .spacing(8)
                    .push(target_content)
                    .push(name_toggler);

                let padded = cosmic::widget::container(content_col).padding(16);
                let popup_content = app.core.applet.popup_container(padded);

                Element::from(popup_content).map(cosmic::Action::App)
            })),
        );

        Task::done(cosmic::Action::Cosmic(cosmic::app::Action::Surface(action)))
    }

    fn send_action(&mut self, action: Action) {
        if let Some(tx) = &mut self.action_tx
            && let Err(error) = tx.try_send(action)
        {
            log::warn!("Dropping Niri action because IPC queue is full: {error}");
        }
    }

    fn capture_session(&self) -> SessionSnapshot {
        let mut outputs: Vec<OutputSnapshot> = Vec::new();
        for workspace_id in &self.ordered_workspace_ids {
            let Some(workspace) = self.raw_workspaces.get(workspace_id) else {
                continue;
            };
            let output_name = workspace.output.clone();
            let output_index = outputs.iter().position(|output| output.name == output_name);
            let output_index = output_index.unwrap_or_else(|| {
                let logical_size = output_name
                    .as_deref()
                    .and_then(|name| self.raw_outputs.get(name))
                    .and_then(|output| output.logical.as_ref())
                    .map(|logical| (logical.width, logical.height));
                outputs.push(OutputSnapshot {
                    name: output_name,
                    width: logical_size.map(|(width, _)| width),
                    height: logical_size.map(|(_, height)| height),
                    workspaces: Vec::new(),
                });
                outputs.len() - 1
            });
            let mut workspace_snapshot = WorkspaceSnapshot {
                id: workspace.id,
                name: workspace.name.clone(),
                index: workspace.idx,
                windows: Vec::new(),
            };
            if let Some(window_ids) = self.windows_by_workspace.get(workspace_id) {
                for window_id in window_ids {
                    let Some(window) = self.raw_windows.get(window_id) else {
                        continue;
                    };
                    let Some(app_id) = window.app_id.clone() else {
                        continue;
                    };
                    let (column, tile) = window.layout.pos_in_scrolling_layout.unwrap_or((1, 1));
                    workspace_snapshot.windows.push(WindowSnapshot {
                        app_id,
                        column,
                        tile,
                        floating: window.is_floating,
                        focused: self.last_focused_window == Some(window.id),
                        width: window.layout.tile_size.0,
                        height: window.layout.tile_size.1,
                    });
                }
            }
            outputs[output_index].workspaces.push(workspace_snapshot);
        }
        SessionSnapshot { outputs }
    }

    fn schedule_session_save(&mut self) -> Task<cosmic::Action<Message>> {
        if !self.session_enabled || self.restoring_session || self.config.fixed_snapshot_enabled {
            return Task::none();
        }
        self.session_generation = self.session_generation.wrapping_add(1);
        let generation = self.session_generation;
        self.latest_session_generation
            .store(generation, Ordering::Release);
        let snapshot = self.capture_session();
        Task::perform(
            async move {
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                (generation, snapshot)
            },
            |(generation, snapshot)| Message::SessionSnapshotReady {
                generation,
                snapshot,
            },
        )
        .map(cosmic::Action::from)
    }

    fn with_session_save(
        &mut self,
        task: Task<cosmic::Action<Message>>,
    ) -> Task<cosmic::Action<Message>> {
        Task::batch(vec![task, self.schedule_session_save()])
    }

    fn begin_session_restore(&mut self) -> Task<cosmic::Action<Message>> {
        let Some(mut snapshot) = self.restore_snapshot.take() else {
            return Task::none();
        };
        if !self.session_enabled {
            return Task::none();
        }

        self.restoring_session = true;
        self.restore_temporary_workspaces.clear();
        let mut temporary_workspace_number = 0usize;
        for output in &mut snapshot.outputs {
            let current_size = output
                .name
                .as_deref()
                .and_then(|name| self.raw_outputs.get(name))
                .and_then(|current| current.logical.as_ref())
                .map(|logical| (logical.width, logical.height));
            if let Some((current_width, current_height)) = current_size {
                for window in output
                    .workspaces
                    .iter_mut()
                    .flat_map(|workspace| &mut workspace.windows)
                {
                    window.width =
                        scaled_restore_dimension(window.width, output.width, current_width);
                    window.height =
                        scaled_restore_dimension(window.height, output.height, current_height);
                }
            }
            if let Some(output_name) = &output.name {
                self.send_action(Action::FocusMonitor {
                    output: output_name.clone(),
                });
            }
            for workspace in &mut output.workspaces {
                self.send_action(Action::FocusWorkspace {
                    reference: niri_ipc::WorkspaceReferenceArg::Index(workspace.index),
                });
                let target_name = workspace.name.clone().unwrap_or_else(|| {
                    temporary_workspace_number += 1;
                    format!(
                        "__cosmic_ext_niri_restore_{}_{}",
                        std::process::id(),
                        temporary_workspace_number
                    )
                });
                self.send_action(Action::SetWorkspaceName {
                    name: target_name.clone(),
                    workspace: Some(niri_ipc::WorkspaceReferenceArg::Index(workspace.index)),
                });
                if workspace.name.is_none() {
                    self.restore_temporary_workspaces.push(target_name.clone());
                }
                workspace.name = Some(target_name);
            }
        }

        let mut used_window_ids = HashSet::new();
        let mut pending = Vec::new();
        let mut focused_window_id = None;
        for output in &snapshot.outputs {
            for workspace in &output.workspaces {
                for window_snapshot in &workspace.windows {
                    let existing_id = self.raw_windows.values().find_map(|window| {
                        (!used_window_ids.contains(&window.id)
                            && window.app_id.as_deref().is_some_and(|app_id| {
                                restore_app_ids_match(app_id, &window_snapshot.app_id)
                            }))
                        .then_some(window.id)
                    });
                    if let Some(window_id) = existing_id {
                        used_window_ids.insert(window_id);
                        self.apply_window_snapshot(window_id, window_snapshot, workspace);
                        if window_snapshot.focused {
                            focused_window_id = Some(window_id);
                        }
                    } else {
                        pending.push((workspace.clone(), window_snapshot.clone()));
                    }
                }
            }
        }
        if let Some(window_id) = focused_window_id {
            self.send_action(Action::FocusWindow { id: window_id });
        }
        self.pending_restore = pending;
        if self.pending_restore.is_empty() {
            Self::session_restore_transition_task()
        } else {
            Self::pending_session_launch_task()
        }
    }

    fn apply_window_snapshot(
        &mut self,
        window_id: u64,
        snapshot: &WindowSnapshot,
        workspace: &WorkspaceSnapshot,
    ) {
        let reference = workspace
            .name
            .clone()
            .map(niri_ipc::WorkspaceReferenceArg::Name)
            .unwrap_or(niri_ipc::WorkspaceReferenceArg::Index(workspace.index));
        self.send_action(Action::MoveWindowToWorkspace {
            window_id: Some(window_id),
            reference,
            focus: false,
        });
        self.send_action(if snapshot.floating {
            Action::MoveWindowToFloating {
                id: Some(window_id),
            }
        } else {
            Action::MoveWindowToTiling {
                id: Some(window_id),
            }
        });
        if snapshot.width > 0.0 {
            self.send_action(Action::SetWindowWidth {
                id: Some(window_id),
                change: niri_ipc::SizeChange::SetFixed(snapshot.width as i32),
            });
        }
        if snapshot.height > 0.0 {
            self.send_action(Action::SetWindowHeight {
                id: Some(window_id),
                change: niri_ipc::SizeChange::SetFixed(snapshot.height as i32),
            });
        }
        self.send_action(Action::FocusWindow { id: window_id });
        if snapshot.column > 0 {
            self.send_action(Action::MoveColumnToIndex {
                index: snapshot.column,
            });
        }
        if snapshot.focused {
            self.send_action(Action::FocusWindow { id: window_id });
        }
    }

    fn restore_new_window(&mut self, window: &Window) -> bool {
        let Some(index) = self.pending_restore.iter().position(|(_, snapshot)| {
            window
                .app_id
                .as_deref()
                .is_some_and(|app_id| restore_app_ids_match(app_id, &snapshot.app_id))
        }) else {
            return false;
        };
        let (workspace, snapshot) = self.pending_restore.remove(index);
        self.apply_window_snapshot(window.id, &snapshot, &workspace);
        self.pending_restore.is_empty()
    }

    fn session_restore_transition_task() -> Task<cosmic::Action<Message>> {
        Task::perform(
            async {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            },
            |_| Message::FinishSessionRestore,
        )
        .map(cosmic::Action::from)
    }

    fn pending_session_launch_task() -> Task<cosmic::Action<Message>> {
        Task::perform(
            async {
                // Desktop autostart and the panel race during login. Give
                // already-starting applications a chance to report their
                // windows before launching the remaining snapshot entries.
                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            },
            |_| Message::LaunchPendingSessionWindows,
        )
        .map(cosmic::Action::from)
    }

    fn update_last_focused_window(&mut self) {
        self.last_focused_window = self
            .raw_windows
            .values()
            .find(|w| w.is_focused)
            .or_else(|| self.raw_windows.values().next())
            .map(|w| w.id);
    }

    fn canonical_app_id(&mut self, app_id: &str) -> String {
        if let Some(canonical) = self.canonical_app_ids.get(app_id) {
            return canonical.clone();
        }

        let canonical = crate::utils::resolve_canonical_desktop_id(app_id);
        self.canonical_app_ids
            .insert(app_id.to_owned(), canonical.clone());
        self.canonical_app_ids
            .entry(canonical.clone())
            .or_insert_with(|| canonical.clone());
        canonical
    }

    fn normalize_window_app_id(&mut self, window: &mut Window) {
        if let Some(app_id) = window.app_id.take() {
            window.app_id = Some(self.canonical_app_id(&app_id));
        }
    }

    /// Rebuild render-only indexes when Niri state changes, never while rendering.
    fn rebuild_workspace_order(&mut self) {
        self.ordered_workspace_ids = self.raw_workspaces.keys().copied().collect();
        self.ordered_workspace_ids.sort_unstable_by_key(|id| {
            self.raw_workspaces
                .get(id)
                .map(|workspace| workspace.idx)
                .unwrap_or(u8::MAX)
        });
    }

    fn rebuild_window_index(&mut self) {
        self.windows_by_workspace.clear();
        self.open_app_ids.clear();

        for window in self.raw_windows.values() {
            if window.app_id.as_deref() == Some(<Self as cosmic::Application>::APP_ID) {
                continue;
            }
            if let Some(workspace_id) = window.workspace_id {
                self.windows_by_workspace
                    .entry(workspace_id)
                    .or_default()
                    .push(window.id);
            }
            if let Some(app_id) = &window.app_id {
                self.open_app_ids.insert(app_id.clone());
            }
        }

        let windows = &self.raw_windows;
        for ids in self.windows_by_workspace.values_mut() {
            ids.sort_unstable_by_key(|id| {
                windows
                    .get(id)
                    .and_then(|window| window.layout.pos_in_scrolling_layout)
                    .unwrap_or((usize::MAX, usize::MAX))
            });
        }
        self.prune_icon_handles();
    }

    fn rebuild_workspace_window_index(&mut self, workspace_id: u64) {
        let mut window_ids: Vec<u64> = self
            .raw_windows
            .values()
            .filter(|window| {
                window.workspace_id == Some(workspace_id)
                    && window.app_id.as_deref() != Some(<Self as cosmic::Application>::APP_ID)
            })
            .map(|window| window.id)
            .collect();
        let windows = &self.raw_windows;
        window_ids.sort_unstable_by_key(|id| {
            windows
                .get(id)
                .and_then(|window| window.layout.pos_in_scrolling_layout)
                .unwrap_or((usize::MAX, usize::MAX))
        });
        if window_ids.is_empty() {
            self.windows_by_workspace.remove(&workspace_id);
        } else {
            self.windows_by_workspace.insert(workspace_id, window_ids);
        }
    }

    fn update_window_indexes_after_change(
        &mut self,
        old_workspace_id: Option<u64>,
        new_workspace_id: Option<u64>,
        old_app_id: Option<String>,
        new_app_id: Option<String>,
    ) {
        if let Some(workspace_id) = old_workspace_id {
            self.rebuild_workspace_window_index(workspace_id);
        }
        if new_workspace_id != old_workspace_id
            && let Some(workspace_id) = new_workspace_id
        {
            self.rebuild_workspace_window_index(workspace_id);
        }

        if old_app_id != new_app_id {
            if let Some(app_id) = old_app_id
                && !self
                    .raw_windows
                    .values()
                    .any(|window| window.app_id.as_deref() == Some(&app_id))
            {
                self.open_app_ids.remove(&app_id);
            }
            if let Some(app_id) = new_app_id
                && app_id != <Self as cosmic::Application>::APP_ID
            {
                self.open_app_ids.insert(app_id);
            }
        }
        self.prune_icon_handles();
    }

    fn prune_icon_handles(&mut self) {
        self.icons.handles.retain(|app_id, _| {
            self.open_app_ids.contains(app_id) || self.pinned.iter().any(|id| id == app_id)
        });
    }

    fn close_popup(&mut self) -> Option<Task<cosmic::Action<Message>>> {
        self.menu.popup_id.take().map(|popup_id| {
            self.menu.target = None;
            Task::done(cosmic::Action::Cosmic(cosmic::app::Action::Surface(
                cosmic::surface::action::destroy_popup(popup_id),
            )))
        })
    }

    fn pre_cache_pinned_desktop_paths(&mut self) {
        for app_id in self.pinned.clone() {
            if !self.desktop_info_cache.contains_key(&app_id)
                && let Some(path) = crate::utils::find_desktop_file_path(&app_id)
            {
                let name =
                    crate::utils::get_desktop_entry_name(&path).unwrap_or_else(|| app_id.clone());
                self.desktop_info_cache
                    .insert(app_id, DesktopAppInfo { path, name });
            }
        }
    }

    /// Unified helper to focus or center windows cleanly without redundant IPC actions
    fn focus_or_center_window(&mut self, window_id: u64, workspace_id: u64, center: bool) {
        self.last_focused_window = Some(window_id);

        if workspace_id > 0 {
            let target_idx = self
                .raw_workspaces
                .get(&workspace_id)
                .map(|w| w.idx)
                .unwrap_or(0);
            self.send_action(Action::FocusWorkspace {
                reference: niri_ipc::WorkspaceReferenceArg::Index(target_idx),
            });
        }
        self.send_action(Action::FocusWindow { id: window_id });

        if center {
            self.send_action(Action::CenterWindow {
                id: Some(window_id),
            });
        }
    }

    /// Calculates current column width percentage for a window from layout dynamically.
    fn calculate_window_width_pct(&self, window_id: u64) -> u32 {
        let w = match self.raw_windows.get(&window_id) {
            Some(w) => w,
            None => return 50,
        };

        let (tile_w, _tile_h) = w.layout.tile_size;

        if tile_w <= 0.0 {
            return 50;
        }

        let output_w = w
            .workspace_id
            .and_then(|ws_id| self.raw_workspaces.get(&ws_id))
            .and_then(|ws| ws.output.as_ref())
            .and_then(|output_name| self.raw_outputs.get(output_name))
            .and_then(|output| output.logical.as_ref())
            .map(|logical| logical.width as f64)
            .filter(|&w| w > 0.0);

        if let Some(w) = output_w {
            let mut pct = (tile_w / w * 100.0).ceil() as u32;
            if pct >= 98 {
                pct = 100;
            }
            return pct.clamp(10, 100);
        }

        50
    }
}

fn fetch_outputs_async() -> Task<Message> {
    Task::perform(
        async {
            if let Ok(mut conn) = Connection::make_connection().await
                && let Ok(Reply::Ok(Response::Outputs(o))) =
                    conn.push_request(Request::Outputs).await
            {
                return Some(o);
            }
            None
        },
        |res| match res {
            Some(o) => Message::OutputsUpdated(o),
            None => Message::Hover(None),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::scaled_restore_dimension;

    #[test]
    fn restore_dimension_tracks_output_resolution() {
        assert_eq!(scaled_restore_dimension(960.0, Some(1920), 2560), 1280.0);
        assert_eq!(scaled_restore_dimension(540.0, Some(1080), 1440), 720.0);
    }

    #[test]
    fn legacy_restore_dimension_remains_absolute() {
        assert_eq!(scaled_restore_dimension(960.0, None, 2560), 960.0);
    }
}
