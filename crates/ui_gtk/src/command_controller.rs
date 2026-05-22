// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::rc::Rc;

use super::{ApplicationCommand, ApplicationRuntimeError, SharedRuntime};
use crate::status_bar::StatusBar;

#[derive(Clone)]
pub(crate) struct UiCommandController {
    runtime: SharedRuntime,
    status_bar: StatusBar,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CommandBatchResult {
    pub(crate) succeeded: usize,
    pub(crate) failed: usize,
    pub(crate) first_error: Option<ApplicationRuntimeError>,
}

impl UiCommandController {
    pub(crate) fn new(runtime: SharedRuntime, status_bar: StatusBar) -> Self {
        Self {
            runtime,
            status_bar,
        }
    }

    pub(crate) fn runtime(&self) -> SharedRuntime {
        self.runtime.clone()
    }

    pub(crate) fn dispatch(
        &self,
        command: ApplicationCommand,
    ) -> Result<(), ApplicationRuntimeError> {
        let result = self.runtime.borrow_mut().handle_command(command);
        match &result {
            Ok(()) => self.status_bar.clear_command_message(),
            Err(error) => self.status_bar.show_command_error(error),
        }
        result
    }

    pub(crate) fn dispatch_succeeded(&self, command: ApplicationCommand) -> bool {
        self.dispatch(command).is_ok()
    }

    pub(crate) fn dispatch_batch(
        &self,
        commands: impl IntoIterator<Item = ApplicationCommand>,
    ) -> CommandBatchResult {
        let mut result = CommandBatchResult {
            succeeded: 0,
            failed: 0,
            first_error: None,
        };

        for command in commands {
            match self.runtime.borrow_mut().handle_command(command) {
                Ok(()) => {
                    result.succeeded += 1;
                }
                Err(error) => {
                    result.failed += 1;
                    if result.first_error.is_none() {
                        result.first_error = Some(error);
                    }
                }
            }
        }

        match (result.succeeded, result.failed, result.first_error.as_ref()) {
            (_, 0, _) => self.status_bar.clear_command_message(),
            (0, _, Some(error)) => self.status_bar.show_command_error(error),
            (_, _, Some(_error)) => self
                .status_bar
                .show_command_message("Some selected tracks could not be updated."),
            (_, _, None) => self.status_bar.clear_command_message(),
        }

        result
    }
}

pub(crate) type SharedCommandController = Rc<UiCommandController>;
