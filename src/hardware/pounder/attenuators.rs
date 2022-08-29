use super::{Channel, Error};

/// Provide an interface for managing digital attenuators on Pounder hardware.
///
/// Note: The digital attenuators do not allow read-back of attenuation. To circumvent this, this
/// driver maintains the attenuation code in both the shift register as well as the latched output
/// register of the attenuators. This allows the "active" attenuation code to be read back by
/// reading the shfit register. The downside of this approach is that any read is destructive, so a
/// read-writeback approach is employed.
pub trait AttenuatorInterface {
    /// Set the attenuation of all pounder channels.
    ///
    /// Args:
    /// * `channel` - A set of channels to configure the attenuation of.
    /// * `attenuations` - The desired attenuation of the channels in dB. This has a resolution of
    ///   0.5dB.
    fn set_attenuations(
        &mut self,
        channels: Channel,
        mut attenuations: [f32; 4],
    ) -> Result<[f32; 4], Error> {
        let mut attenuation_codes = [0; 4];
        let mut bytes = [0; 4];
        for (i, att) in attenuations.iter().enumerate() {
            if !(0.0..=31.5).contains(att) {
                return Err(Error::Bounds);
            }

            // Calculate the attenuation code to program into the attenuator. The attenuator uses a
            // code where the LSB is 0.5 dB.
            attenuation_codes[i] = (att * 2.0) as u8;

            // The lowest 2 bits of the 8-bit shift register on the attenuator are ignored. Shift the
            // attenuator code into the upper 6 bits of the register value. Note that the attenuator
            // treats inputs as active-low, so the code is inverted before writing.
            bytes[i] = !(attenuation_codes[i] << 2);
        }

        // Configure attenuations of all channels at the same time.
        self.transfer_attenuators(&mut bytes)?;

        // Finally, latch the output of the updated channel to force it into an active state.
        self.latch_attenuators(channels)?;

        for i in 0..4 {
            attenuations[i] = attenuation_codes[i] as f32 / 2.0;
        }

        Ok(attenuations)
    }

    fn reset_attenuators(&mut self) -> Result<(), Error>;

    fn latch_attenuators(&mut self, channel: Channel) -> Result<(), Error>;

    fn transfer_attenuators(
        &mut self,
        channels: &mut [u8; 4],
    ) -> Result<(), Error>;
}
