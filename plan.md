# Plan

## Goal

Extract runtime policy and state transitions out of GPUI so timers, playback behavior, transfer behavior, and persistence are driven by a testable core instead of UI entities.

## What Should Move Out Of GPUI

### Playback Core

- Extract a pure playback session state machine.
- Own loaded track state, paused/playing/buffering/idle state, pending play requests, resume position, repeat, shuffle, track-finished handling, and playback-failed handling.
- Move this logic out of the current mix of:
  - `src/app/playback/state.rs`
  - `src/app/controller.rs`
  - `src/app/playback/view.rs`

### Transfer Core

- Extract a pure transfer lifecycle model.
- Represent queued, active, paused, completed, and failed transfers explicitly.
- Keep "visible as active in UI" separate from "needs periodic refresh".
- Move this logic out of GPUI-driven refresh behavior and center it around a domain model derived from:
  - `src/app/transfer_state.rs`

### Refresh Policy

- Extract refresh scheduling into a pure policy module.
- Given playback state and transfer state, answer:
  - whether periodic ticks are needed
  - why they are needed
  - when they can stop
- Remove refresh policy decisions from:
  - `src/app.rs`

### Domain Events And Reducer

- Define app-domain events such as:
  - `PlayPressed`
  - `PausePressed`
  - `StopPressed`
  - `RouteChanged`
  - `TrackFinished`
  - `TransferProgressed`
  - `ExternalDownloadPaused`
  - `ExternalDownloadResumed`
- Route those through a reducer/effect planner instead of scattering transitions across UI handlers.

### Session Snapshot Mapping

- Persist and restore from core state structs, not directly from live GPUI entities.
- Move persistence decisions behind a pure state-to-snapshot mapping.
- Reduce coupling in:
  - `src/app/session_state.rs`

## What GPUI Should Keep

- Rendering
- Focus and input handling
- Subscriptions to backend events
- Spawning concrete timers/tasks requested by core policy
- Translating core effects into:
  - audio calls
  - transfer calls
  - notifications
  - persistence

## Incremental Extraction Order

1. Extract `PlaybackStateModel` into a core module with no GPUI types.
2. Extract `TransferStateModel` into a core module with no GPUI types.
3. Extract `RefreshPolicy` as a pure module.
4. Make `OryxApp` hold core state and adapt it into GPUI rendering.
5. Move event handling to a reducer plus effects layer.

## Why

- Prevent UI-framework structure from accidentally defining runtime behavior.
- Make timer behavior explicit and testable.
- Reduce bugs like:
  - paused downloads keeping repaint loops alive
  - route-change recovery depending on incidental UI polling shape
  - playback behavior being split across multiple GPUI-driven handlers
