use crate::xml::{field, optional_field};

pub struct Radio {
    pub wlan_index: u32,
}

pub const RADIO0_5G: Radio = Radio { wlan_index: 0 };
pub const RADIO1_2G: Radio = Radio { wlan_index: 1 };

fn byte_field(hs: &str, key: &str) -> Result<u8, String> {
    let value = field(hs, key)?;
    let parsed = value
        .trim()
        .trim_start_matches("0x")
        .parse::<u32>()
        .map_err(|_| format!("invalid numeric field {key}: {value}"))?;
    u8::try_from(parsed).map_err(|_| format!("field {key} out of byte range: {value}"))
}

fn hex_bytes_field(hs: &str, key: &str) -> Result<Vec<u8>, String> {
    let value = field(hs, key)?;
    value
        .split(',')
        .map(|part| u8::from_str_radix(part.trim(), 16).map_err(|_| format!("invalid hex field {key}: {value}")))
        .collect()
}

fn mac_address(base: [u8; 6], offset: u32) -> [u8; 6] {
    let value = u64::from(base[0]) << 40
        | u64::from(base[1]) << 32
        | u64::from(base[2]) << 24
        | u64::from(base[3]) << 16
        | u64::from(base[4]) << 8
        | u64::from(base[5]);
    let shifted = value + u64::from(offset);
    [
        (shifted >> 40) as u8,
        (shifted >> 32) as u8,
        (shifted >> 24) as u8,
        (shifted >> 16) as u8,
        (shifted >> 8) as u8,
        shifted as u8,
    ]
}

pub fn flash_calibration(hs: &str, radio: &Radio, base_mac: [u8; 6]) -> Result<Vec<u8>, String> {
    let mut data = Vec::with_capacity(386);
    data.extend_from_slice(b"TYFC0003");
    for key in [
        "HW_RX_GAINGAP_2G_CCK_AB",
        "HW_RX_GAINGAP_2G_OFDM_AB",
        "HW_RX_GAINGAP_5GL_AB",
        "HW_RX_GAINGAP_5GM_AB",
        "HW_RX_GAINGAP_5GH_AB",
    ] {
        data.push(byte_field(hs, key)?);
    }
    for path in ["A", "B"] {
        data.push(byte_field(hs, &format!("HW_THERMAL_{path}_WLAN{}", radio.wlan_index))?);
    }
    data.push(byte_field(hs, &format!("HW_XCAP_WLAN{}_AX", radio.wlan_index))?);
    for band in ["2G_CCK", "2G_BW40_1S", "5G_BW40_1S"] {
        for path in ["A", "B"] {
            data.extend_from_slice(&hex_bytes_field(hs, &format!("HW_{band}_TSSI_{path}"))?);
        }
    }
    let address = mac_address(base_mac, 2 - radio.wlan_index);
    data.extend_from_slice(&address);
    for kind in ["GAIN", "CW"] {
        for path in ["A", "B"] {
            data.extend_from_slice(&hex_bytes_field(
                hs,
                &format!("HW_2G_CCK_TSSI_{kind}_DIFF_{path}"),
            )?);
            for band in ["2G", "5G"] {
                data.extend_from_slice(&hex_bytes_field(
                    hs,
                    &format!("HW_{band}_TSSI_{kind}_DIFF_{path}"),
                )?);
            }
        }
    }
    let slope = hex_bytes_field(hs, "HW_5G_TSSI_CW_DIFF_A")?;
    let slope = *slope.last().ok_or("empty slope field")?;
    for seed in [0u8, slope] {
        for width in 0u8..4 {
            for _ in 0..14 {
                data.push(if width == 1 { 0 } else { seed });
            }
        }
    }
    Ok(data)
}

pub fn epon_llid_mac(config_xml: &str) -> Option<String> {
    let start = config_xml.find("<chain chainName=\"EPON_LLID_TBL\">")?;
    let remainder = &config_xml[start..];
    let end = remainder.find("</chain>")?;
    let chain = &remainder[..end];
    optional_field(chain, "macAddr").filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_offset_carry() {
        let base = [0xd0, 0x5b, 0xcb, 0x9c, 0x9b, 0xf0];
        assert_eq!(mac_address(base, 2), [0xd0, 0x5b, 0xcb, 0x9c, 0x9b, 0xf2]);
        let max = [0x00, 0x00, 0x00, 0x00, 0x00, 0xff];
        assert_eq!(mac_address(max, 2), [0x00, 0x00, 0x00, 0x00, 0x01, 0x01]);
    }

    #[test]
    fn hex_lists() {
        let doc = r#"<Value Name="HW_2G_CCK_TSSI_A" Value="fe,fe,09,04,04,04"/>"#;
        assert_eq!(
            hex_bytes_field(doc, "HW_2G_CCK_TSSI_A").unwrap(),
            vec![0xfe, 0xfe, 0x09, 0x04, 0x04, 0x04]
        );
    }
}
