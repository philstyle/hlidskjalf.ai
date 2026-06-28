use crate::db::DbState;
use crate::tailscale::TailscaleService;
use crate::types::NexusLinkStatus;

pub fn get_nexuslink_status(
    db: &DbState,
    ts: &TailscaleService,
) -> Result<NexusLinkStatus, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;

    // Get config values
    let instance_key: String = conn
        .query_row(
            "SELECT value FROM nexuslink_config WHERE key = 'instance_key'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("Instance key not found: {}", e))?;

    let port: String = conn
        .query_row(
            "SELECT value FROM nexuslink_config WHERE key = 'server_port'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "4242".to_string());

    let paired_device_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM paired_devices WHERE revoked = 0",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    drop(conn); // Release DB lock

    // Re-resolve Tailscale IP live (may differ from static server bind_addr)
    let (tailscale_ip, tailscale_error) = match ts.get_ip() {
        Ok(ip) => (Some(ip), None),
        Err(e) => (None, Some(e)),
    };

    // Generate QR code if we have a Tailscale IP
    let qr_svg = tailscale_ip.as_ref().map(|ip| {
        let content = format!("http://{}:{}/#{}", ip, port, instance_key);
        generate_qr_svg(&content)
    });

    // Determine bind address for display
    let bind_address = if let Some(ref ip) = tailscale_ip {
        format!("{}:{}", ip, port)
    } else {
        format!("127.0.0.1:{}", port)
    };

    Ok(NexusLinkStatus {
        running: true,
        bind_address,
        tailscale_ip,
        tailscale_error,
        qr_svg,
        paired_device_count,
    })
}

/// Generate a QR code as an SVG string (light-on-dark for SkyNexus aesthetic).
fn generate_qr_svg(content: &str) -> String {
    use qrcode::QrCode;

    let code = match QrCode::new(content.as_bytes()) {
        Ok(c) => c,
        Err(_) => return String::new(),
    };

    let module_count = code.width();
    let quiet_zone = 2;
    let total = module_count + quiet_zone * 2;
    let module_size = 4; // px per module
    let svg_size = total * module_size;

    let mut svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {} {}" width="{}" height="{}">"#,
        svg_size, svg_size, svg_size, svg_size
    );

    // Dark background
    svg.push_str(&format!(
        r##"<rect width="{}" height="{}" fill="#0a0a0a"/>"##,
        svg_size, svg_size
    ));

    // Light modules (#e0e0e0 on dark bg)
    let colors = code.to_colors();
    for (i, color) in colors.iter().enumerate() {
        if *color == qrcode::Color::Dark {
            let row = i / module_count;
            let col = i % module_count;
            let x = (col + quiet_zone) * module_size;
            let y = (row + quiet_zone) * module_size;
            svg.push_str(&format!(
                r##"<rect x="{}" y="{}" width="{}" height="{}" fill="#e0e0e0"/>"##,
                x, y, module_size, module_size
            ));
        }
    }

    svg.push_str("</svg>");
    svg
}
