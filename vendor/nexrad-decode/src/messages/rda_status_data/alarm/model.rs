/// An RDA alarm message definition to be referenced by an RDA status data message.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Message {
    code: u16,
    state: Option<State>,
    alarm_type: Option<AlarmType>,
    device: Option<Device>,
    sample: Option<u8>,
    message: &'static str,
}

impl Message {
    pub(crate) fn new(
        code: u16,
        state: Option<State>,
        alarm_type: Option<AlarmType>,
        device: Option<Device>,
        sample: Option<u8>,
        message: &'static str,
    ) -> Self {
        Self {
            code,
            state,
            alarm_type,
            device,
            sample,
            message,
        }
    }

    /// The alarm code.
    pub fn code(&self) -> u16 {
        self.code
    }

    /// The status of the RDA as a result of the alarm.
    pub fn state(&self) -> Option<State> {
        self.state
    }

    /// The type of alarm.
    pub fn alarm_type(&self) -> Option<AlarmType> {
        self.alarm_type
    }

    /// The hardware device area where the alarm originated.
    pub fn device(&self) -> Option<Device> {
        self.device
    }

    /// The number of samples required to trigger the alarm.
    pub fn sample(&self) -> Option<u8> {
        self.sample
    }

    /// The alarm message.
    pub fn message(&self) -> &'static str {
        self.message
    }
}

/// The status of the RDA as a result of the alarm.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum State {
    /// Maintenance is mandatory before operation can continue.
    MaintenanceMandatory,
    /// Maintenance is required but operation can continue.
    MaintenanceRequired,
    /// System is inoperative due to this alarm.
    Inoperative,
    /// Alarm not specifically tied to state change.
    Secondary,
}

/// The different classifications of alarms.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum AlarmType {
    /// Alarm failed consecutively enough times to meet the alarm reporting count/sample threshold.
    EdgeDetected,
    /// Alarm reported each time the condition is met.
    Occurrence,
    /// Alarm reported at most once every 15 minutes when the condition is met.
    FilteredOccurrence,
}

/// The hardware device area where the alarm originated.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum Device {
    /// Control unit/computer.
    Control,
    /// Antenna pedestal.
    Pedestal,
    /// Receiver subsystem.
    Receiver,
    /// Signal processor.
    SignalProcessor,
    /// Communications equipment.
    Communications,
    /// Tower utilities (power, HVAC, etc.).
    TowerUtilities,
    /// Transmitter subsystem.
    Transmitter,
}
