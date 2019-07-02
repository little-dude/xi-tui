use std::collections::HashMap;
use std::io::Write;

use futures::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use futures::{Async, Future, Poll, Stream};

use failure::Error;
use indexmap::IndexMap;
use termion::event::{Event, Key};
use serde_json::Value;

use xrl::{Client, ConfigChanged, ScrollTo, Style, Update, ViewId, XiNotification};

use crate::core::{Command, CoreEvent, KeyMap};

use crate::widgets::{View, ViewClient};

#[derive(Debug)]
pub enum XiReply {
    NewView((ViewId, Option<String>)),
    CopiedText(Option<String>),
}

/// The main interface to xi-core
pub struct Editor {
    /// Channel from which the responses to "new_view" requests are
    /// received. Upon receiving a `ViewId`, the `Editdor` creates a
    /// new view.
    pub xi_reply_rx: UnboundedReceiver<XiReply>,

    /// Channel into which the responses to "new_view" requests are
    /// sent, when they are received from the core.
    pub xi_reply_tx: UnboundedSender<XiReply>,

    /// Store the events that we cannot process right away.
    ///
    /// Due to the asynchronous nature of the communication with the
    /// core, we may receive events we cannot process on the
    /// moment. For instance, when opening a new view, we may receive
    /// notifications for it whereas we are not even done processing
    /// the response to the "open" request, and hence, the view might
    /// not even be created on our side yet.
    pub delayed_events: Vec<CoreEvent>,

    /// The views that are opened.
    pub views: IndexMap<ViewId, View>,

    /// Id of the view that is currently displayed on screen.
    pub current_view: ViewId,

    /// A client to send notifications or request to `xi-core`.
    pub client: Client,

    pub size: (u16, u16),
    pub styles: HashMap<u64, Style>,

    pub keymap: KeyMap,
    clipboard: Option<String>,
}

/// Methods for general use.
impl Editor {
    pub fn new(client: Client, keymap: KeyMap) -> Editor {
        let mut styles = HashMap::new();
        styles.insert(0, Default::default());
        let (xi_reply_tx, xi_reply_rx) = mpsc::unbounded::<XiReply>();

        Editor {
            xi_reply_rx,
            xi_reply_tx,
            delayed_events: Vec::new(),
            views: IndexMap::new(),
            current_view: ViewId(0),
            client,
            size: (0, 0),
            styles,
            keymap,
            clipboard: None,
        }
    }
}

// Strictly speaking we don't have to implement Future for the editor,
// because we don't spawn it on the tokio runtime. But I'm still
// somewhat undecided whether we should or not, and having the editor
// implemented as a Future will make things easier if we decide to go
// that way.
impl Future for Editor {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        debug!("polling the editor");

        debug!("handling delayed events");
        if !self.delayed_events.is_empty() {
            let delayed_events: Vec<CoreEvent> = self.delayed_events.drain(..).collect();
            for event in delayed_events {
                self.handle_core_event(event);
            }
        }

        debug!("polling 'new_view' responses");
        loop {
            match self.xi_reply_rx.poll() {
                Ok(Async::Ready(Some(XiReply::NewView((view_id, file_path))))) => {
                    info!("creating new view {:?}", view_id);
                    let client = ViewClient::new(self.client.clone(), view_id);
                    let mut view = View::new(client, file_path);
                    view.resize(self.size.1);
                    self.views.insert(view_id, view);
                    info!("switching to view {:?}", view_id);
                    self.current_view = view_id;
                }

                Ok(Async::Ready(Some(XiReply::CopiedText(text)))) => {
                    info!("Got new text for clipboard {:?}", text);
                    self.clipboard = text;
                }

                // We own one of the senders so this cannot happen
                Ok(Async::Ready(None)) => unreachable!(),
                Ok(Async::NotReady) => {
                    debug!("no more 'new_view' response");
                    break;
                }
                Err(e) => {
                    error!("Unkown channel error: {:?}", e);
                    return Err(());
                }
            }
        }
        Ok(Async::NotReady)
    }
}

impl Editor {
    /// Handle keyboard and mouse events
    pub fn handle_input(&mut self, event: Event) {
        match event {
            Event::Mouse(mouse_event) => self.views.get_mut(&self.current_view).unwrap().handle_mouse_event(mouse_event),            
            ev => {
                match self.keymap.get(&ev).cloned() {
                    Some(cmd) => self.handle_command(cmd),
                    None => { 
                        if let Some(view) = self.views.get_mut(&self.current_view) {
                            match ev {
                                Event::Key(Key::Char(c)) => view.handle_command(Command::Insert(c)),
                                k => error!("un-handled key {:?}", k)
                            }
                        }
                    }
                }
            },
        }
    }

    pub fn handle_command(&mut self, cmd: Command) {
        match cmd {
            Command::SetTheme(theme) => self.set_theme(&theme),
            Command::NextBuffer => self.next_buffer(),
            Command::PrevBuffer => self.prev_buffer(),
            Command::Save(view_id) => self.save(view_id),
            Command::Open(file) => self.new_view(file),
            Command::CloseCurrentView => self.close_view(None),
            Command::CopySelection => self.copy(),
            Command::CutSelection => self.cut(),
            Command::Paste => self.paste(),
            view_command => {
                        if let Some(view) = self.views.get_mut(&self.current_view) {
                            view.handle_command(view_command)
                        }
            }
        }
    }

    /// Handle terminal size changes
    pub fn handle_resize(&mut self, size: (u16, u16)) {
        info!("setting new terminal size");
        self.size = size;
        if let Some(view) = self.views.get_mut(&self.current_view) {
            view.resize(size.1);
        } else {
            warn!("view {} not found", self.current_view);
        }
    }

    /// Handle message from xi-core, that the TUI forwarded us.
    pub fn handle_core_event(&mut self, event: CoreEvent) {
        match event {
            CoreEvent::Notify(notification) => match notification {
                XiNotification::Update(update) => self.update(update),
                XiNotification::DefStyle(style) => self.def_style(style),
                XiNotification::ScrollTo(scroll_to) => self.scroll_to(scroll_to),
                XiNotification::ConfigChanged(config) => self.config_changed(config),
                _ => info!("ignoring Xi core notification: {:?}", notification),
            },
            CoreEvent::MeasureWidth((_request, _result_tx)) => unimplemented!(),
        }
    }

    /// Handle an "update" notification from Xi core.
    fn update(&mut self, update: Update) {
        match self.views.get_mut(&update.view_id) {
            Some(view) => view.update_cache(update),
            None => self
                .delayed_events
                .push(CoreEvent::Notify(XiNotification::Update(update))),
        }
    }

    /// Handle a "scroll_to" notification from Xi core.
    fn scroll_to(&mut self, scroll_to: ScrollTo) {
        match self.views.get_mut(&scroll_to.view_id) {
            Some(view) => view.set_cursor(scroll_to.line, scroll_to.column),
            None => self
                .delayed_events
                .push(CoreEvent::Notify(XiNotification::ScrollTo(scroll_to))),
        }
    }

    /// Handle a "def_style" notification from Xi core.
    fn def_style(&mut self, style: Style) {
        self.styles.insert(style.id, style);
    }

    /// Handle a "config_changed" notification from Xi core.
    fn config_changed(&mut self, config: ConfigChanged) {
        match self.views.get_mut(&config.view_id) {
            Some(view) => view.config_changed(config.changes),
            None => self
                .delayed_events
                .push(CoreEvent::Notify(XiNotification::ConfigChanged(config))),
        }
    }

    /// Spawn a future that sends a "new_view" request to the core,
    /// and forwards the response back to the `Editor`.
    pub fn close_view(&mut self, view_id: Option<ViewId>) {
        if self.views.len() <= 1 {
            // We don't close the last view.
            // TODO: Exit the editor instead
            return;
        }

        let mut closed = false;
        let view_to_close = view_id.unwrap_or(self.current_view);
        if let Some(view) = self.views.get_mut(&view_to_close) {
            view.handle_command(Command::CloseCurrentView);
            closed = true;
        }
        if closed {
            self.prev_buffer();
            self.views.remove(&view_to_close);
        }
    }

    /// Spawn a future that sends a "copy" request to the core,
    /// and forwards the response back to the `Editor`.
    fn copy(&mut self) {
        let response_tx = self.xi_reply_tx.clone();
        if let Some(view) = self.views.get_mut(&self.current_view) {
            let future = view.copy()
                                .and_then(move |x| {
                                    // when we get the response from the core, forward the copied
                                    // text to the editor so that the clipboard can be filled/replaced
                                    let text = match x {
                                               Value::String(s) => Some(s),
                                               z => { error!("ERROR when parsing copy-answer: Wrong type. {:?}", z); None },
                                    };
                                    response_tx
                                        .unbounded_send(XiReply::CopiedText(text))
                                        .unwrap_or_else(|e| error!("failed to send \"CopiedText\" response: {:?}", e));
                                    Ok(())
                                })
                                .or_else(|client_error| {
                                    error!("failed to send \"CopiedText\" response: {:?}", client_error);
                                    Ok(())
                                });
            tokio::spawn(future);
        }
    }


    /// Spawn a future that sends a "cut" request to the core,
    /// and forwards the response back to the `Editor`.
    fn cut(&mut self) {
        let response_tx = self.xi_reply_tx.clone();
        if let Some(view) = self.views.get_mut(&self.current_view) {
            let future = view.cut()
                                .and_then(move |x| {
                                    // when we get the response from the core, forward the copied
                                    // text to the editor so that the clipboard can be filled/replaced
                                    let text = match x {
                                               Value::String(s) => Some(s),
                                               z => { error!("ERROR when parsing cut-answer: Wrong type. {:?}", z); None },
                                    };
                                    response_tx
                                        .unbounded_send(XiReply::CopiedText(text))
                                        .unwrap_or_else(|e| error!("failed to send \"CopiedText\" response: {:?}", e));
                                    Ok(())
                                })
                                .or_else(|client_error| {
                                    error!("failed to send \"CopiedText\" response: {:?}", client_error);
                                    Ok(())
                                });
            tokio::spawn(future);
        }
    }

    // Paste clipboard
    fn paste(&mut self) {
        if let Some(view) = self.views.get_mut(&self.current_view) {
            match self.clipboard {
                Some(ref content) => view.paste(content),
                None => {}
            };
        }
    }

    /// Spawn a future that sends a "new_view" request to the core,
    /// and forwards the response back to the `Editor`.
    pub fn new_view(&mut self, file_path: Option<String>) {
        let response_tx = self.xi_reply_tx.clone();
        let future = self
            .client
            .new_view(file_path.clone())
            .and_then(move |id| {
                // when we get the response from the core, forward the new
                // view id to the editor so that the view can be created
                response_tx
                    .unbounded_send(XiReply::NewView((id, file_path)))
                    .unwrap_or_else(|e| error!("failed to send \"new_view\" response: {:?}", e));
                Ok(())
            })
            .or_else(|client_error| {
                error!("failed to send \"new_view\" response: {:?}", client_error);
                Ok(())
            });
        tokio::spawn(future);
    }

    /// Spawn a future that sends a "set_theme" notification to the
    /// core for the current view.
    pub fn set_theme(&mut self, theme: &str) {
        tokio::spawn(self.client.set_theme(theme).map_err(|_| ()));
    }

    /// Spawn a future that sends a "save" notification to the core.
    pub fn save(&mut self, view_id: Option<ViewId>) {
        match self.views.get_mut(&view_id.unwrap_or(self.current_view)) {
            Some(view) => view.save(),
            None => warn!("cannot save view {:?}: not found", &view_id),
        }
    }

    pub fn next_buffer(&mut self) {
        if let Some((dex, _, _)) = self.views.get_full(&self.current_view) {
            if dex + 1 == self.views.len() {
                if let Some((view, _)) = self.views.get_index(0) {
                    self.current_view = *view;
                }
            } else if let Some((view, _)) = self.views.get_index(dex + 1) {
                self.current_view = *view;
            }
        }
    }

    pub fn prev_buffer(&mut self) {
        if let Some((dex, _, _)) = self.views.get_full(&self.current_view) {
            if dex == 0 {
                if let Some((view, _)) = self.views.get_index(self.views.len() - 1) {
                    self.current_view = *view;
                }
            } else if let Some((view, _)) = self.views.get_index(dex - 1) {
                self.current_view = *view;
            }
        }
    }
}

/// Methods ment to be called by the tui struct
impl Editor {
    // We render if:
    //  - the current view is dirty
    //  - we switched views
    //  - the style changed
    //  - the terminal size changed
    pub fn render<W: Write>(&mut self, term: &mut W) -> Result<(), Error> {
        if let Some(view) = self.views.get_mut(&self.current_view) {
            debug!("rendering the current view");
            view.render(term, &self.styles)?;
        } else {
            warn!("no view to render");
        }
        Ok(())
    }
}
