//! The connect screen shown at launch: RIT's login panel for Bunting
//! (`docs/research/rit-binary-audit/desk-client-inventory-1.8.466.md`,
//! "Onboarding"). BNP identity is a registered certificate, never a
//! password, so the choices are the practice market on this machine or a
//! hosted venue joined with a one-time invite (ADR 0056: the key is made
//! here) or with operator-issued certificate files. The last choice
//! is remembered, Enter connects, and a failure leaves the form filled in
//! with the reason under it. Flags (`--local`, `--server`, ...) skip this
//! screen, so scripts behave as before.

use crate::connect::{self, Options};
use crate::theme;
use crate::ui;
use bunting_trading_state::enrolment::enroll_with_invite;
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, IntoElement, ParentElement as _,
    PathPromptOptions, Render, SharedString, Styled as _, Subscription, Window, div, px,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where the desk connects.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// The shared practice market on this machine.
    #[default]
    Local,
    /// A venue hosted elsewhere, reached over BNP with a certificate.
    Hosted,
}

/// The connect screen's fields, saved after every attempt so the next
/// launch opens on the same choice.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Choice {
    pub mode: Mode,
    /// A local participant to claim (1-8); the next free one when unset.
    pub participant: Option<u64>,
    pub server: String,
    pub server_name: String,
    pub ca: String,
    pub cert: String,
    pub key: String,
}

impl Choice {
    /// The saved choice at `path`, or the default when there is none or
    /// it cannot be read.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Saves the choice at `path`, creating its directory.
    ///
    /// # Errors
    /// Returns the I/O error when the file cannot be written.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, text)
    }

    /// The connection options this choice asks for, on top of `base`
    /// (which keeps the flags this screen does not show, such as the
    /// server program and the venue directory).
    ///
    /// # Errors
    /// Returns what is missing or malformed, in the words of the form.
    pub fn options(&self, base: &Options) -> Result<Options, String> {
        let mut options = base.clone();
        let text = |value: &str| {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_owned())
        };
        match self.mode {
            Mode::Local => {
                if let Some(participant) = self.participant
                    && !(1..=8).contains(&participant)
                {
                    return Err("Participant must be 1 to 8, or empty for the next free one".into());
                }
                options.local = true;
                options.server = None;
                options.participant = self.participant;
            }
            Mode::Hosted => {
                let required = |value: &str, name: &str| {
                    text(value).ok_or_else(|| format!("{name} is required for a hosted venue"))
                };
                options.local = false;
                options.participant = None;
                options.server = Some(required(&self.server, "Address")?);
                if !options
                    .server
                    .as_deref()
                    .is_some_and(|server| server.contains(':'))
                {
                    return Err("Address must be host:port".into());
                }
                options.server_name = text(&self.server_name);
                options.ca = Some(PathBuf::from(required(&self.ca, "CA certificate")?));
                options.cert = Some(PathBuf::from(required(&self.cert, "Your certificate")?));
                options.key = Some(PathBuf::from(required(&self.key, "Your key")?));
            }
        }
        Ok(options)
    }
}

/// Where an identity enrolled at `address` as `participant_id` is kept:
/// one directory per venue and participant under the desk's config.
fn identity_dir(address: &str, participant_id: u128) -> PathBuf {
    let venue: String = address
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    crate::appearance::desk_config_dir()
        .join("identities")
        .join(format!("{venue}-participant-{participant_id}"))
}

/// The saved connect choice's default location, beside the workspaces.
#[must_use]
pub fn default_choice_path() -> PathBuf {
    crate::layouts::LayoutStore::default_location()
        .path()
        .with_file_name("connection.json")
}

#[derive(Clone, Debug)]
enum Phase {
    Idle,
    Connecting(SharedString),
    Failed(SharedString),
}

/// One text field of the form.
struct Field {
    state: Entity<InputState>,
}

impl Field {
    fn new(
        value: &str,
        placeholder: &'static str,
        window: &mut Window,
        cx: &mut Context<Onboarding>,
    ) -> Self {
        let value = value.to_owned();
        Self {
            state: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(value)
            }),
        }
    }

    fn text(&self, cx: &gpui_kit::App) -> String {
        self.state.read(cx).value().trim().to_owned()
    }
}

/// The connect screen.
pub struct Onboarding {
    base: Options,
    store: PathBuf,
    mode: Mode,
    participant: Field,
    server: Field,
    server_name: Field,
    ca: Field,
    cert: Field,
    key: Field,
    /// A one-time invite (ADR 0056); never saved.
    invite: Field,
    phase: Phase,
    _enter: Vec<Subscription>,
}

impl Onboarding {
    pub fn new(base: Options, store: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let saved = Choice::load(&store);
        let participant = saved
            .participant
            .map(|id| id.to_string())
            .unwrap_or_default();
        let fields = [
            Field::new(&participant, "Next free", window, cx),
            Field::new(&saved.server, "host:port", window, cx),
            Field::new(&saved.server_name, "Same as the host", window, cx),
            Field::new(&saved.ca, "ca.pem", window, cx),
            Field::new(&saved.cert, "participant.pem", window, cx),
            Field::new(&saved.key, "participant.key", window, cx),
            Field::new("", "bnpi1:… from your venue operator", window, cx),
        ];
        let enter = fields
            .iter()
            .map(|field| {
                cx.subscribe_in(
                    &field.state,
                    window,
                    |this, _, event: &InputEvent, window, cx| {
                        if matches!(event, InputEvent::PressEnter { .. }) {
                            this.connect(window, cx);
                        }
                    },
                )
            })
            .collect();
        let [participant, server, server_name, ca, cert, key, invite] = fields;
        Self {
            base,
            store,
            mode: saved.mode,
            participant,
            server,
            server_name,
            ca,
            cert,
            key,
            invite,
            phase: Phase::Idle,
            _enter: enter,
        }
    }

    fn choice(&self, cx: &gpui_kit::App) -> Result<Choice, String> {
        let participant = self.participant.text(cx);
        let participant = if participant.is_empty() {
            None
        } else {
            Some(
                participant
                    .parse::<u64>()
                    .map_err(|_| "Participant must be a number from 1 to 8".to_owned())?,
            )
        };
        Ok(Choice {
            mode: self.mode,
            participant,
            server: self.server.text(cx),
            server_name: self.server_name.text(cx),
            ca: self.ca.text(cx),
            cert: self.cert.text(cx),
            key: self.key.text(cx),
        })
    }

    fn connecting(&self) -> bool {
        matches!(self.phase, Phase::Connecting(_))
    }

    /// Validates the form, saves it, and connects off the UI thread. On
    /// success the trading workspace opens and this window closes.
    fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connecting() {
            return;
        }
        let invite = self.invite.text(cx);
        if self.mode == Mode::Hosted && !invite.is_empty() {
            self.join_with_invite(invite, window, cx);
            return;
        }
        let options = match self.choice(cx).and_then(|choice| {
            let options = choice.options(&self.base)?;
            if let Err(error) = choice.save(&self.store) {
                eprintln!(
                    "bunting-desk: cannot save {}: {error}",
                    self.store.display()
                );
            }
            Ok(options)
        }) {
            Ok(options) => options,
            Err(problem) => {
                self.phase = Phase::Failed(problem.into());
                cx.notify();
                return;
            }
        };
        self.phase = Phase::Connecting(match self.mode {
            Mode::Local => "Starting the practice market…".into(),
            Mode::Hosted => format!(
                "Connecting to {}…",
                options.server.as_deref().unwrap_or_default()
            )
            .into(),
        });
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let opened = cx
                .background_spawn(async move { connect::open(&options) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| match opened {
                Ok((venue, label)) => match crate::open_desk_window(venue, label, cx) {
                    Ok(()) => window.remove_window(),
                    Err(error) => {
                        this.phase = Phase::Failed(error.into());
                        cx.notify();
                    }
                },
                Err(error) => {
                    this.phase = Phase::Failed(sentence(&error).into());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Enrols with `invite` (ADR 0056) off the UI thread: a key is created
    /// here, the venue issues its certificate, the identity is saved under
    /// the desk's config directory and remembered, and the desk connects.
    fn join_with_invite(&mut self, invite: String, window: &mut Window, cx: &mut Context<Self>) {
        self.phase = Phase::Connecting("Joining with your invite…".into());
        cx.notify();
        let (base, store) = (self.base.clone(), self.store.clone());
        cx.spawn_in(window, async move |this, cx| {
            let opened = cx
                .background_spawn(async move {
                    let identity = enroll_with_invite(&invite)?;
                    let directory = identity_dir(&identity.address, identity.participant_id);
                    identity.save(&directory).map_err(|error| {
                        format!(
                            "cannot save your identity in {}: {error}",
                            directory.display()
                        )
                    })?;
                    let file = |name: &str| directory.join(name).display().to_string();
                    let choice = Choice {
                        mode: Mode::Hosted,
                        participant: None,
                        server: identity.address.clone(),
                        server_name: identity.server_name.clone(),
                        ca: file("ca.pem"),
                        cert: file("participant.pem"),
                        key: file("participant.key"),
                    };
                    if let Err(error) = choice.save(&store) {
                        eprintln!("bunting-desk: cannot save {}: {error}", store.display());
                    }
                    connect::open(&choice.options(&base)?)
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| match opened {
                Ok((venue, label)) => match crate::open_desk_window(venue, label, cx) {
                    Ok(()) => window.remove_window(),
                    Err(error) => {
                        this.phase = Phase::Failed(error.into());
                        cx.notify();
                    }
                },
                Err(error) => {
                    this.phase = Phase::Failed(sentence(&error).into());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Fills `which` from the platform's file chooser.
    fn pick_file(which: FileField, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = prompt.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                let field = match which {
                    FileField::Ca => this.ca.state.clone(),
                    FileField::Cert => this.cert.state.clone(),
                    FileField::Key => this.key.state.clone(),
                };
                field.update(cx, |state, cx| {
                    state.set_value(path.display().to_string(), window, cx);
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// The fields of the chosen mode.
    fn body(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        match self.mode {
            Mode::Local => v_flex()
                .gap_3()
                .child(ui::muted(
                    "Trade in the shared market on this machine, with built-in traders",
                    cx,
                ))
                .child(ui::field(
                    "Participant",
                    div()
                        .w(px(120.))
                        .child(Input::new(&self.participant.state).small())
                        .into_any_element(),
                    cx,
                )),
            Mode::Hosted => v_flex()
                .gap_3()
                .child(ui::field(
                    "Invite",
                    Input::new(&self.invite.state).small().into_any_element(),
                    cx,
                ))
                .child(ui::muted("Or join with certificate files", cx))
                .child(ui::field(
                    "Address",
                    Input::new(&self.server.state).small().into_any_element(),
                    cx,
                ))
                .child(ui::field(
                    "Certificate name",
                    Input::new(&self.server_name.state)
                        .small()
                        .into_any_element(),
                    cx,
                ))
                .child(self.file_row(FileField::Ca, "CA certificate", cx))
                .child(self.file_row(FileField::Cert, "Your certificate", cx))
                .child(self.file_row(FileField::Key, "Your key", cx)),
        }
    }

    fn file_row(
        &self,
        which: FileField,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let field = match which {
            FileField::Ca => &self.ca,
            FileField::Cert => &self.cert,
            FileField::Key => &self.key,
        };
        ui::field(
            label,
            h_flex()
                .items_center()
                .gap_2()
                .child(div().flex_1().child(Input::new(&field.state).small()))
                .child(
                    Button::new(which.id())
                        .small()
                        .label("Browse…")
                        .disabled(self.connecting())
                        .on_click(cx.listener(move |_, _, window, cx| {
                            Self::pick_file(which, window, cx);
                        })),
                )
                .into_any_element(),
            cx,
        )
        .into_any_element()
    }
}

/// The certificate fields that take a file.
#[derive(Clone, Copy, Debug)]
enum FileField {
    Ca,
    Cert,
    Key,
}

impl FileField {
    const fn id(self) -> &'static str {
        match self {
            Self::Ca => "connect-browse-ca",
            Self::Cert => "connect-browse-cert",
            Self::Key => "connect-browse-key",
        }
    }
}

/// Errors read as clauses on a terminal; on screen they are sentences.
fn sentence(message: &str) -> String {
    let mut chars = message.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

impl Render for Onboarding {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (background, foreground, muted, danger) = (
            theme.background,
            theme.foreground,
            theme.muted_foreground,
            theme.danger,
        );
        let busy = self.connecting();
        let mode = self.mode;
        let mode_button = |id: &'static str, label: &'static str, which: Mode| {
            Button::new(id)
                .label(label)
                .selected(mode == which)
                .flex_1()
        };
        let modes = ButtonGroup::new("connect-mode")
            .small()
            .w_full()
            .disabled(busy)
            .child(mode_button(
                "connect-local",
                "Practice locally",
                Mode::Local,
            ))
            .child(mode_button("connect-hosted", "Hosted venue", Mode::Hosted))
            .on_click(cx.listener(|this, selected: &Vec<usize>, _, cx| {
                this.mode = if selected.first() == Some(&1) {
                    Mode::Hosted
                } else {
                    Mode::Local
                };
                this.phase = Phase::Idle;
                cx.notify();
            }));
        let body = self.body(cx);
        let status = match &self.phase {
            Phase::Idle => None,
            Phase::Connecting(text) => Some((text.clone(), muted)),
            Phase::Failed(text) => Some((text.clone(), danger)),
        };
        div()
            .size_full()
            .bg(background)
            .text_color(foreground)
            .flex()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .w(px(420.))
                    .p_6()
                    .gap_4()
                    .child(theme::serif("Bunting").text_size(px(22.)))
                    .child(modes)
                    .child(body)
                    .when_some(status, |this, (text, color)| {
                        this.child(div().text_sm().text_color(color).child(text))
                    })
                    .child(
                        h_flex().justify_end().child(
                            Button::new("connect-go")
                                .primary()
                                .small()
                                .label("Connect")
                                .loading(busy)
                                .disabled(busy)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.connect(window, cx)),
                                ),
                        ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_choice_claims_the_named_participant_or_the_next_free() {
        let base = Options::default();
        let choice = Choice {
            participant: Some(3),
            ..Choice::default()
        };
        let options = choice.options(&base).unwrap_or_default();
        assert!(options.local && options.server.is_none());
        assert_eq!(options.participant, Some(3));
        let bad = Choice {
            participant: Some(9),
            ..Choice::default()
        };
        assert!(bad.options(&base).is_err());
    }

    #[test]
    fn a_hosted_choice_needs_its_address_and_every_file() {
        let mut choice = Choice {
            mode: Mode::Hosted,
            server: "100.113.213.48:7400".into(),
            ca: "ca.pem".into(),
            cert: "me.pem".into(),
            ..Choice::default()
        };
        assert!(choice.options(&Options::default()).is_err(), "key missing");
        choice.key = "me.key".into();
        let options = choice.options(&Options::default()).unwrap_or_default();
        assert!(!options.is_local());
        assert_eq!(options.server.as_deref(), Some("100.113.213.48:7400"));
        assert_eq!(options.server_name, None, "defaults to the host");
        choice.server = "no-port".into();
        assert!(choice.options(&Options::default()).is_err());
    }

    #[test]
    fn the_choice_survives_a_save_and_a_load() -> std::io::Result<()> {
        let directory = std::env::temp_dir().join(format!("desk-choice-{}", std::process::id()));
        let path = directory.join("connection.json");
        let choice = Choice {
            mode: Mode::Hosted,
            server: "venue:7400".into(),
            ..Choice::default()
        };
        choice.save(&path)?;
        assert_eq!(Choice::load(&path), choice);
        assert_eq!(
            Choice::load(&directory.join("absent.json")),
            Choice::default()
        );
        std::fs::remove_dir_all(&directory)
    }
}
