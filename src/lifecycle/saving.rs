//! Saving the open World: one save at a time, sampled after the frame's focus has gone to the
//! simulation, captured and written off the frame.
//!
//! A save samples the player and takes its place in the store's line ([`WorldStore::ticket`]) in
//! `PostUpdate`, after the frame's focus has gone to the simulation ([`SendFocus`]), then asks the
//! simulation for a capture. The capture is taken after every command sent before it, so it holds
//! the focus the player was sampled with. Once it arrives, another thread encodes and writes it.
//! Until that write ends, a save asked for waits: the saves of one World commit in the order they
//! were asked for.
//!
//! [`SendFocus`]: crate::sim_bridge::SendFocus

use std::time::{Duration, SystemTime};

use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use crossbeam_channel::{Receiver, TryRecvError};

use super::{CurrentWorld, Leaving, Library, WallClock, go};
use crate::player::Player;
use crate::saves::{PlayerState, SaveTicket, StoreError, WorldSave, WorldSummary};
use crate::session::Play;
use crate::settings::{AutosaveInterval, Settings};
use crate::sim_bridge::{SimBridge, SimCapture};

/// When the open World saves by itself: once it has run for `interval` since it last saved.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Autosave {
    /// The player's choice in Settings ([`Settings::autosave`]).
    pub interval: Duration,
    /// Unpaused play since the World last saved, or since the latest autosave was tried. Time
    /// while the World is paused doesn't count.
    pub since_save: Duration,
}

impl Default for Autosave {
    fn default() -> Self {
        Self {
            interval: AutosaveInterval::default().duration(),
            since_save: Duration::ZERO,
        }
    }
}

/// Why the World saves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SaveReason {
    Autosave,
    /// The player asked.
    Manual,
    /// The player is leaving the World.
    Leaving,
}

/// How the latest save of the open World ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SaveStatus {
    /// None has ended since the World opened.
    #[default]
    None,
    /// It committed, at `at` on the app's real-time clock (`Time<Real>::elapsed`).
    Saved { reason: SaveReason, at: Duration },
    /// It failed, for this reason. The World's last save stays as it was.
    Failed {
        reason: SaveReason,
        message: String,
        at: Duration,
    },
}

/// The open World's saves: the one under way, the one waiting, and how the latest ended.
#[derive(Resource, Default)]
pub struct Saving {
    /// How the latest save ended.
    pub last: SaveStatus,
    job: Option<SaveJob>,
    /// The save to start once the one under way ends. Saves asked for meanwhile merge into one.
    waiting: Option<SaveReason>,
}

impl Saving {
    /// The save under way, or waiting to start, if any.
    pub fn in_progress(&self) -> Option<SaveReason> {
        self.job.as_ref().map(|job| job.reason).or(self.waiting)
    }

    /// Asks for a save. It starts at the end of the frame, or once the one under way ends. Asked
    /// for while another waits, the two become one save, for the weightier reason.
    pub(super) fn request(&mut self, reason: SaveReason) {
        self.waiting = self.waiting.max(Some(reason));
    }

    /// Forgets every save, as the World closes. A write under way still finishes.
    pub(super) fn reset(&mut self) {
        if let Some(SaveJob {
            stage: Stage::Writing(task),
            ..
        }) = self.job.take()
        {
            task.detach();
        }
        *self = Self::default();
    }

    /// Moves the save under way along, and returns how it ended, once it has.
    fn poll(&mut self, library: &Library) -> Option<(SaveReason, Result<WorldSummary, String>)> {
        let SaveJob { reason, stage } = self.job.take()?;
        let stage = match stage {
            Stage::Capturing {
                ticket,
                player,
                now,
                capture,
            } => match capture.try_recv() {
                Ok(capture) => {
                    let Ok(store) = library.store().cloned() else {
                        return Some((reason, Err("there is nowhere to save Worlds".into())));
                    };
                    Stage::Writing(AsyncComputeTaskPool::get().spawn(async move {
                        store.save(ticket, &WorldSave { capture, player }, now)
                    }))
                }
                Err(TryRecvError::Empty) => Stage::Capturing {
                    ticket,
                    player,
                    now,
                    capture,
                },
                Err(TryRecvError::Disconnected) => {
                    let message = "the simulation stopped before it could be captured";
                    return Some((reason, Err(message.into())));
                }
            },
            Stage::Writing(mut task) => match check_ready(&mut task) {
                Some(written) => return Some((reason, written.map_err(|error| error.to_string()))),
                None => Stage::Writing(task),
            },
        };
        self.job = Some(SaveJob { reason, stage });
        None
    }
}

/// A save under way.
struct SaveJob {
    reason: SaveReason,
    stage: Stage,
}

enum Stage {
    /// Waiting for the simulation's capture.
    Capturing {
        ticket: SaveTicket,
        player: PlayerState,
        /// The last-played time to save.
        now: SystemTime,
        capture: Receiver<SimCapture>,
    },
    /// Encoding and writing on another thread.
    Writing(Task<Result<WorldSummary, StoreError>>),
}

/// Takes up the autosave interval the player chose. Play counted already still counts: a shorter
/// interval that has passed already autosaves at once.
pub(super) fn follow_autosave_setting(settings: Res<Settings>, mut autosave: ResMut<Autosave>) {
    let interval = settings.autosave.duration();
    if autosave.interval != interval {
        autosave.interval = interval;
    }
}

/// Counts unpaused play, and asks for an autosave once it is due and no other save is under way.
/// Trying it restarts the count, so a save that fails is tried again only an interval later.
pub(super) fn count_play_time(
    time: Res<Time<Real>>,
    mut autosave: ResMut<Autosave>,
    mut saving: ResMut<Saving>,
) {
    autosave.since_save += time.delta();
    if autosave.since_save >= autosave.interval && saving.in_progress().is_none() {
        saving.request(SaveReason::Autosave);
        autosave.since_save = Duration::ZERO;
    }
}

/// Moves the save under way along, takes in how it ended, and starts the next.
#[expect(clippy::too_many_arguments, reason = "a system's parameters")]
pub(super) fn run_saves(
    mut saving: ResMut<Saving>,
    mut autosave: ResMut<Autosave>,
    mut leaving: ResMut<Leaving>,
    current: Option<ResMut<CurrentWorld>>,
    (library, clock, time): (Res<Library>, Res<WallClock>, Res<Time<Real>>),
    bridge: Res<SimBridge>,
    play: Res<State<Play>>,
    player: Query<(&Player, &Transform)>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(mut current) = current else {
        // Only a World opened from the main menu saves.
        saving.waiting = None;
        return;
    };
    if let Some((reason, ended)) = saving.poll(&library) {
        let at = time.elapsed();
        saving.last = match ended {
            Ok(summary) => {
                info!("saved World {} ({reason:?})", summary.id);
                current.0 = summary;
                autosave.since_save = Duration::ZERO;
                SaveStatus::Saved { reason, at }
            }
            Err(message) => {
                error!("the World didn't save ({reason:?}): {message}");
                SaveStatus::Failed {
                    reason,
                    message,
                    at,
                }
            }
        };
        if reason == SaveReason::Leaving
            && let Some(departure) = &mut leaving.0
        {
            match &saving.last {
                SaveStatus::Saved { .. } => go(departure.to, &mut commands, &mut exit),
                SaveStatus::Failed { message, .. } => departure.failed = Some(message.clone()),
                SaveStatus::None => {}
            }
        }
    }

    let Some(reason) = saving.waiting.filter(|_| saving.job.is_none()) else {
        return;
    };
    // The save for leaving waits until the World has stopped, so it holds the World as left.
    if reason == SaveReason::Leaving && *play.get() == Play::Running {
        return;
    }
    let (Ok(store), Ok((body, transform))) = (library.store(), player.single()) else {
        return;
    };
    saving.waiting = None;
    saving.job = Some(SaveJob {
        reason,
        stage: Stage::Capturing {
            ticket: store.ticket(&current.0.id),
            player: PlayerState::new(**body, transform.rotation),
            now: clock.now(),
            capture: bridge.capture(),
        },
    });
}
