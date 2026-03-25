# Validation Proposal

This proposal outlines how to validate playback, streaming, and cache correctness beyond unit tests. The main goal is to catch lifecycle, integration, and fault-handling bugs that only show up in the full app.

## Problem

Unit tests are useful for local logic, but they are weak at proving correctness for:

- sleep and wake behavior
- media key integration
- output device invalidation
- streaming stalls and partial reads
- cache promotion and reuse
- long-running state corruption

These issues need process-level and system-level validation.

## Proposed Validation Layers

### 1. Fault-Injection Provider Harness

Build a local HTTP/provider harness that can deliberately serve bad or unstable playback responses while the real app runs against it.

Support scenarios such as:

- close the stream after `N` bytes
- advertise a `Content-Length` larger than the delivered body
- stall reads for a long interval
- fail the first request and succeed on retry
- return complete audio with inconsistent metadata
- hang forever unless the client enforces timeouts

This gives deterministic reproduction for the failure classes that currently escape normal tests.

### 2. Process-Level Scenario Tests

Run the app as a real process and drive it through realistic playback flows. Do not stop at calling internal functions.

Assert on observable outcomes such as:

- playback status transitions
- status messages
- pending play/loading flags
- transfer/download state
- cache database rows
- temporary and final files on disk
- no persistent wedged states after failure

This is the highest-value automated layer after the fault harness exists.

### 3. Cache Integrity Audits

In test mode, validate every downloaded audio file before and after promotion into the cache.

Check:

- file duration is within tolerance
- parsed format and bitrate are readable when expected
- cache metadata matches parsed file properties
- no `.part` files remain after completion or failure
- no cache rows point to missing or invalid audio files

This should also be runnable as a standalone audit over an existing library/cache.

### 4. Soak and Stress Runs

Add long-running scripted playback sessions that repeatedly exercise:

- play, pause, resume
- next and previous spam
- download plus playback concurrency
- intermittent network failures
- app restart during active transfers
- repeated use of the same cached tracks

The purpose is to catch state leaks, stuck workers, and recovery failures that short tests miss.

### 5. Sleep/Wake and Device Invalidation Certification

Keep a dedicated validation pass for macOS lifecycle behavior.

Cover:

- pause, sleep overnight, wake, resume
- output device switch while paused
- output device switch while playing
- media key control after wake
- recovery after stream invalidation or device loss

This likely needs a manual or semi-automated certification checklist, because full automation here is difficult and brittle.

### 6. Structured Runtime Telemetry

Add structured logs for critical state transitions with stable identifiers.

Include fields like:

- play request id
- track id
- playback state
- worker command
- stream error
- download state
- cache validation result

This makes failures diagnosable and also enables future automated assertions over event traces.

### 7. Runtime Invariants in Debug/Test Builds

Add assertions for conditions that should never hold.

Examples:

- `play_loading` must clear after a request resolves or fails
- failed progressive downloads must not be treated as `TrackFinished`
- cached files must validate before reuse
- `Playing` must not exist without an active current track
- paused and resumed state must keep a valid resume position

Invariants catch whole classes of regressions that normal examples can miss.

### 8. Release Candidate Manual Checklist

Maintain a short high-risk checklist to run before shipping.

Suggested flows:

- sleep and wake while paused
- sleep and wake while playing
- network drop mid-stream
- start a different track after stream failure
- interrupted download and retry
- replay a previously cached track after a failed stream
- verify a collection with mixed qualities does not contain fake low-bitrate labels from broken files
- switch output devices during playback

This is intentionally small and focused on the highest-risk behaviors.

## Suggested Order of Implementation

Recommended sequence:

1. Fault-injection provider harness
2. Process-level scenario runner
3. Structured playback and transfer telemetry
4. Cache integrity audit command
5. Soak suite
6. Manual certification checklist

## Expected Outcome

If implemented, this validation stack should make playback correctness much more trustworthy than unit tests alone. The main improvement is deterministic reproduction and verification of real-world failure modes rather than relying on personal use to discover regressions.
