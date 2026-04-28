//! SIP002 `ss://` URL assembly and QR PNG encoding for a single SS user.
//!
//! Shadowsocks 2022 with EIH expects the client password to be the user's
//! iPSK concatenated with the server's uPSK, separated by `:`. The URL is
//! rendered directly (method + percent-encoded password + host:port +
//! fragment) because upstream's `ServerConfig::to_url` only emits the uPSK.

use std::net::IpAddr;

use base64::{engine::general_purpose::STANDARD_NO_PAD as B64, Engine as _};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use qrcodegen::{QrCode, QrCodeEcc};
use shadowsocks_service::shadowsocks::crypto::CipherKind;

use crate::error::SsError;

/// Inputs required to emit a per-user client config.
pub struct ClientConfig<'a> {
    pub name: &'a str,
    pub host: &'a str,
    pub port: u16,
    pub method: CipherKind,
    pub server_psk: &'a [u8; crate::driver::PSK_LEN],
    pub user_psk: &'a [u8; crate::driver::PSK_LEN],
}

/// Render an `ss://` URL in SIP002 userinfo form.
///
/// For AEAD-2022 with EIH the userinfo password is `iPSK_b64:uPSK_b64`,
/// percent-encoded per RFC 3986 (no base64 of the whole userinfo because
/// AEAD-2022 keeps method + password plain). Upstream's `ServerConfig::to_url`
/// stores only the uPSK after parsing and therefore cannot emit the iPSK; we
/// build the URL directly instead.
pub fn build_ss_url(cfg: &ClientConfig<'_>) -> Result<String, SsError> {
    let method = cfg.method.to_string();
    let password_plain = format!(
        "{}:{}",
        B64.encode(cfg.user_psk),
        B64.encode(cfg.server_psk)
    );
    let password_enc = utf8_percent_encode(&password_plain, NON_ALPHANUMERIC).to_string();
    let host = render_host(cfg.host)?;
    let remark_enc = utf8_percent_encode(cfg.name, NON_ALPHANUMERIC).to_string();
    Ok(format!(
        "ss://{method}:{password_enc}@{host}:{port}#{remark_enc}",
        port = cfg.port
    ))
}

/// Hosts in an `ss://` URL can be either IP or DNS. IPv6 literals must be
/// wrapped in `[..]` per RFC 3986; IPv4 and DNS names are emitted as-is.
fn render_host(host: &str) -> Result<String, SsError> {
    if host.is_empty() {
        return Err(SsError::Invalid("empty host".to_owned()));
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V6(v6)) => Ok(format!("[{v6}]")),
        _ => Ok(host.to_owned()),
    }
}

/// Render a PNG QR code that encodes the `ss://` URL at error-correction level M.
///
/// The bitmap is scaled to roughly 256 px wide. We encode grayscale 1-bit
/// rather than RGBA to keep the payload small.
pub fn build_ss_qr_png(url: &str) -> Result<Vec<u8>, SsError> {
    let code = QrCode::encode_text(url, QrCodeEcc::Medium)
        .map_err(|e| SsError::Config(format!("qr encode: {e}")))?;
    let size = code.size();
    let border: i32 = 2;
    let scale: usize = compute_scale(size as usize, border as usize, 256);
    let img_side_modules = (size as usize) + (border as usize * 2);
    let img_side_px = img_side_modules * scale;

    // 1 byte per pixel, grayscale (0 = black, 255 = white). We invert so QR
    // "dark" modules become zeros.
    let mut pixels = vec![255u8; img_side_px * img_side_px];
    for py in 0..img_side_px {
        for px in 0..img_side_px {
            let mx = (px / scale) as i32 - border;
            let my = (py / scale) as i32 - border;
            if code.get_module(mx, my) {
                pixels[py * img_side_px + px] = 0;
            }
        }
    }

    let mut png_bytes = Vec::with_capacity(pixels.len() / 4);
    let width_u32 = u32::try_from(img_side_px)
        .map_err(|e| SsError::Config(format!("qr width overflow: {e}")))?;
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, width_u32, width_u32);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| SsError::Config(format!("png header: {e}")))?;
        writer
            .write_image_data(&pixels)
            .map_err(|e| SsError::Config(format!("png data: {e}")))?;
    }
    Ok(png_bytes)
}

fn compute_scale(module_count: usize, border: usize, target_px: usize) -> usize {
    let total_modules = module_count + border * 2;
    (target_px / total_modules.max(1)).max(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aead2022_128() -> CipherKind {
        "2022-blake3-aes-128-gcm".parse().unwrap()
    }

    #[test]
    fn url_uses_ss_scheme_and_host() {
        let upsk = [7u8; crate::driver::PSK_LEN];
        let ipsk = [11u8; crate::driver::PSK_LEN];
        let url = build_ss_url(&ClientConfig {
            name: "alice",
            host: "proxy.example.com",
            port: 4433,
            method: aead2022_128(),
            server_psk: &upsk,
            user_psk: &ipsk,
        })
        .expect("url");
        assert!(url.starts_with("ss://"), "got: {url}");
        assert!(url.contains("proxy.example.com"), "got: {url}");
        assert!(url.ends_with("#alice"), "got: {url}");
        // The password carries both keys separated by `:`, percent-encoded.
        assert!(url.contains("%3A"), "got: {url}");
    }

    #[test]
    fn url_with_ipv4_host_inlines_socket_addr() {
        let upsk = [1u8; crate::driver::PSK_LEN];
        let ipsk = [2u8; crate::driver::PSK_LEN];
        let url = build_ss_url(&ClientConfig {
            name: "u",
            host: "127.0.0.1",
            port: 8388,
            method: aead2022_128(),
            server_psk: &upsk,
            user_psk: &ipsk,
        })
        .expect("url");
        assert!(url.contains("127.0.0.1:8388"), "got: {url}");
    }

    /// Round-trip: build the URL, hand it back to shadowsocks-rust's own
    /// parser, confirm the PSK bytes decode correctly. Covers all
    /// shadowsocks-rust-based clients (Outline, ClashX-rs, v2rayN, …).
    #[test]
    fn url_parses_back_with_shadowsocks_rust() {
        use shadowsocks_service::shadowsocks::config::ServerConfig;

        let upsk = [7u8; crate::driver::PSK_LEN];
        let ipsk = [11u8; crate::driver::PSK_LEN];
        let url = build_ss_url(&ClientConfig {
            name: "alice",
            host: "proxy.example.com",
            port: 4433,
            method: aead2022_128(),
            server_psk: &upsk,
            user_psk: &ipsk,
        })
        .unwrap();

        let parsed = ServerConfig::from_url(&url).expect("parse ss:// back");
        assert_eq!(parsed.method(), aead2022_128());
        assert_eq!(parsed.addr().to_string(), "proxy.example.com:4433");
        // SIP022 URL password is `iPSK:uPSK`; shadowsocks-rust returns the
        // trailing uPSK (our server_psk) as the ServerConfig main key.
        assert_eq!(parsed.key(), &upsk[..]);
    }

    #[test]
    fn qr_encodes_png_magic() {
        let upsk = [9u8; crate::driver::PSK_LEN];
        let ipsk = [3u8; crate::driver::PSK_LEN];
        let url = build_ss_url(&ClientConfig {
            name: "u",
            host: "127.0.0.1",
            port: 4433,
            method: aead2022_128(),
            server_psk: &upsk,
            user_psk: &ipsk,
        })
        .unwrap();
        let png = build_ss_qr_png(&url).expect("png");
        assert!(
            png.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]),
            "not a PNG: first 8 = {:?}",
            &png[..8]
        );
    }
}
