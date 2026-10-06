# Aurora Field Notes

Field observations from the *Aurora* environmental monitoring project,
station **K-7**, winter season. All readings are collected by the station's
sensor array and logged locally before sync.

> Note: calibration values below are placeholders from the bench test and
> will be replaced after the first full deployment cycle.

## Overview

The station runs twelve sensor channels sampled at 1 Hz. Each sensor
reports through the `telemetry` bus, and a nightly job aggregates the raw
readings into hourly summaries. Configuration lives in `station.toml` and
is reloaded without a restart.

## Readings

### Temperature

| Channel | Sensor  | Min (°C) | Max (°C) | Status |
|---------|---------|---------:|---------:|--------|
| A1      | ext-01  |   -23.4  |   -11.2  | ok     |
| A2      | ext-02  |   -24.1  |   -12.8  | ok     |
| B1      | soil-01 |    -2.3  |     0.4  | drift  |
| B2      | soil-02 |    -2.0  |     0.1  | ok     |

The drift flag on `soil-01` is expected: the probe was resited last week
and the baseline has not been relearned yet.

### Wind

Anemometer samples are debounced in the driver; gusts are reported as a
90-second rolling peak. The vane offset correction is `+3.2°` after the
mount realignment.

### Power

Battery charge ended the month at **87%**, down from 91% in November. The
heater duty cycle dominates consumption during cold snaps.

## Checklist

- [x] Swap the failed sensor relay on channel B1
- [x] Re-run the baseline calibration job
- [ ] Replace the anti-ice wick on the wet bulb
- [ ] Archive the November raw logs to cold storage
- [ ] Schedule the spring antenna inspection

## Software

The station firmware is built with the usual toolchain:

```rust
fn poll_all(channels: &mut [Channel]) -> Summary {
    let readings: Vec<Reading> = channels
        .iter_mut()
        .filter_map(|ch| ch.sample())
        .collect();
    Summary::from(readings)
}
```

Retries use exponential backoff with jitter; the sync client gives up
after `6` attempts and queues the batch for the next window.

## Archive

The 2025 log rotation policy ~~deletes~~ retains raw logs for 18 months.
Compressed summaries are kept indefinitely.
