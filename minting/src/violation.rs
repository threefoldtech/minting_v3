use std::fmt;

#[derive(Debug, Default)]
pub enum Violation {
    /// No violation detected
    #[default]
    None,
    /// This node reported an uptime which increased more than the timestamp since last reported
    /// (accounting for the uptime grace period).
    UptimeTooHigh {
        previous_uptime: u64,
        reported_uptime: u64,
        previous_timestamp: i64,
        reported_timestamp: i64,
        block_reported: u32,
    },
    /// This node reported an uptime which increased compared to the last reported uptime, is high
    /// enough to not be considered a reboot, and is also lower than expected for a regular turned
    /// on node.
    UptimeTooLow {
        previous_uptime: u64,
        reported_uptime: u64,
        previous_timestamp: i64,
        reported_timestamp: i64,
        block_reported: u32,
    },
    /// The node is known to be rebooted, but the uptime reported would idicate it rebooted before
    /// the previous uptime report.
    InvalidReboot {
        previous_uptime: u64,
        reported_uptime: u64,
        previous_timestamp: i64,
        reported_timestamp: i64,
        block_reported: u32,
    },
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Violation::None => f.pad(""),
            Violation::UptimeTooHigh { previous_uptime, reported_uptime, previous_timestamp, reported_timestamp, block_reported } => f.write_fmt(format_args!("Node uptime increased more than time increased! Previous datapoint ({previous_timestamp}, {previous_uptime}), new datapoint ({reported_timestamp}, {reported_uptime}) in block {block_reported}")),
            Violation::UptimeTooLow { previous_uptime, reported_uptime, previous_timestamp, reported_timestamp, block_reported } => f.write_fmt(format_args!("Node uptime increased less than time increased, and node was not rebooted! Previous datapoint ({previous_timestamp}, {previous_uptime}), new datapoint ({reported_timestamp}, {reported_uptime}) in block {block_reported}")),
            Violation::InvalidReboot { previous_uptime, reported_uptime, previous_timestamp, reported_timestamp, block_reported } => f.write_fmt(format_args!("Node rebooted before the previous uptime report! Previous datapoint ({previous_timestamp}, {previous_uptime}), new datapoint ({reported_timestamp}, {reported_uptime}) in block {block_reported}")),
        }
    }
}

impl Violation {
    pub fn is_none(&self) -> bool {
        matches!(self, Violation::None)
    }

    pub fn is_some(&self) -> bool {
        !self.is_none()
    }
}
