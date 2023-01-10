///! Stabilizer network management module
///!
///! # Design
///! The stabilizer network architecture supports numerous layers to permit transmission of
///! telemetry (via MQTT), configuration of run-time settings (via MQTT + Miniconf), and live data
///! streaming over raw UDP/TCP sockets. This module encompasses the main processing routines
///! related to Stabilizer networking operations.
pub use heapless;
pub use miniconf;
pub use serde;

pub mod data_stream;
pub mod network_processor;
pub mod telemetry;

use crate::hardware::{EthernetPhy, NetworkManager, NetworkStack, SystemTimer};
use data_stream::{DataStream, FrameGenerator};
use minimq::embedded_nal::IpAddr;
use network_processor::NetworkProcessor;
use telemetry::TelemetryClient;

use core::fmt::Write;
use heapless::String;
use miniconf::Miniconf;
use serde::Serialize;
use smoltcp_nal::embedded_nal::SocketAddr;

pub type NetworkReference =
    smoltcp_nal::shared::NetworkStackProxy<'static, NetworkStack>;

/// The default MQTT broker IP address if unspecified.
pub const DEFAULT_MQTT_BROKER: [u8; 4] = [10, 34, 16, 10];

pub enum UpdateState {
    NoChange,
    Updated,
}

pub enum NetworkState {
    SettingsChanged(String<128>),
    Updated,
    NoChange,
}

/// A structure of Stabilizer's default network users.
pub struct NetworkUsers<S: Miniconf + Clone, T: Serialize> {
    pub miniconf: miniconf::MqttClient<S, NetworkReference, SystemTimer, 512>,
    pub processor: NetworkProcessor,
    stream: DataStream,
    generator: Option<FrameGenerator>,
    pub telemetry: TelemetryClient<T>,
}

impl<S, T> NetworkUsers<S, T>
where
    S: Miniconf + Clone,
    T: Serialize,
{
    /// Construct Stabilizer's default network users.
    ///
    /// # Args
    /// * `stack` - The network stack that will be used to share with all network users.
    /// * `phy` - The ethernet PHY connecting the network.
    /// * `clock` - A `SystemTimer` implementing `Clock`.
    /// * `app` - The name of the application.
    /// * `mac` - The MAC address of the network.
    /// * `broker` - The IP address of the MQTT broker to use.
    /// * `settings` - The initial settings value
    ///
    /// # Returns
    /// A new struct of network users.
    pub fn new(
        stack: NetworkStack,
        phy: EthernetPhy,
        clock: SystemTimer,
        app: &str,
        mac: smoltcp_nal::smoltcp::wire::EthernetAddress,
        broker: IpAddr,
        settings: S,
    ) -> Self {
        let stack_manager =
            cortex_m::singleton!(: NetworkManager = NetworkManager::new(stack))
                .unwrap();

        let processor =
            NetworkProcessor::new(stack_manager.acquire_stack(), phy);

        let prefix = get_device_prefix(app, mac);

        let settings = miniconf::MqttClient::new(
            stack_manager.acquire_stack(),
            &get_client_id(app, "settings", mac),
            &prefix,
            broker,
            clock,
            settings,
        )
        .unwrap();

        let telemetry = TelemetryClient::new(
            stack_manager.acquire_stack(),
            clock,
            &get_client_id(app, "tlm", mac),
            &prefix,
            broker,
        );

        let (generator, stream) =
            data_stream::setup_streaming(stack_manager.acquire_stack());

        NetworkUsers {
            miniconf: settings,
            processor,
            telemetry,
            stream,
            generator: Some(generator),
        }
    }

    /// Enable live data streaming.
    ///
    /// # Args
    /// * `format` - A unique u8 code indicating the format of the data.
    pub fn configure_streaming(
        &mut self,
        format: impl Into<u8>,
        batch_size: u8,
    ) -> FrameGenerator {
        let mut generator = self.generator.take().unwrap();
        generator.configure(format, batch_size);
        generator
    }

    /// Direct the stream to the provided remote target.
    ///
    /// # Args
    /// * `remote` - The destination for the streamed data.
    pub fn direct_stream(&mut self, remote: SocketAddr) {
        if self.generator.is_none() {
            self.stream.set_remote(remote);
        }
    }

    /// Update and process all of the network users state.
    ///
    /// # Returns
    /// An indication if any of the network users indicated a state change.
    /// The SettingsChanged option contains the path of the settings that changed.
    pub fn update(&mut self) -> NetworkState {
        // Update the MQTT clients.
        self.telemetry.update();

        // Update the data stream.
        if self.generator.is_none() {
            self.stream.process();
        }

        // Poll for incoming data.
        let poll_result = match self.processor.update() {
            UpdateState::NoChange => NetworkState::NoChange,
            UpdateState::Updated => NetworkState::Updated,
        };

        // `settings_path` has to be at least as large as `miniconf::mqtt_client::MAX_TOPIC_LENGTH`.
        let mut settings_path: String<128> = String::new();
        match self.miniconf.handled_update(|path, old, new| {
            settings_path = path.into();
            *old = new.clone();
            Result::<(), &'static str>::Ok(())
        }) {
            Ok(true) => NetworkState::SettingsChanged(settings_path),
            _ => poll_result,
        }
    }
}

/// Get an MQTT client ID for a client.
///
/// # Args
/// * `app` - The name of the application
/// * `client` - The unique tag of the client
/// * `mac` - The MAC address of the device.
///
/// # Returns
/// A client ID that may be used for MQTT client identification.
fn get_client_id(
    app: &str,
    client: &str,
    mac: smoltcp_nal::smoltcp::wire::EthernetAddress,
) -> String<64> {
    let mut identifier = String::new();
    write!(&mut identifier, "{}-{}-{}", app, mac, client).unwrap();
    identifier
}

/// Get the MQTT prefix of a device.
///
/// # Args
/// * `app` - The name of the application that is executing.
/// * `mac` - The ethernet MAC address of the device.
///
/// # Returns
/// The MQTT prefix used for this device.
pub fn get_device_prefix(
    app: &str,
    mac: smoltcp_nal::smoltcp::wire::EthernetAddress,
) -> String<128> {
    // Note(unwrap): The mac address + binary name must be short enough to fit into this string. If
    // they are defined too long, this will panic and the device will fail to boot.
    let mut prefix: String<128> = String::new();
    write!(&mut prefix, "dt/sinara/{}/{}", app, mac).unwrap();

    prefix
}
