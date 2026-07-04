#[cfg(windows)]
use std::process::Command;

#[cfg(windows)]
pub fn install_printer() -> Result<(), String> {
    // Requiere privilegios de administrador para crear puertos/impresoras.
    // Utiliza los cmdlets de PrintManagement: Add-PrinterPort / Add-Printer.
    let script = r#"
$ErrorActionPreference = 'Stop'
$portName = 'ESCPosViewer9100'
$printerName = 'ESCPos Viewer (TCP 9100)'

try {
    # 1. Buscar drivers compatibles (soporte para Windows en inglés y español)
    # Prioridad: Texto Genérico (RAW) -> PDF (fallback) -> Búsqueda por patrón
    $driversToTry = @(
        'Generic / Text Only',
        'Genérico / Solo texto',
        'Microsoft Print to PDF',
        'Microsoft imprimir en PDF'
    )
    $driverName = $null

    # Intento por nombres exactos conocidos
    foreach ($d in $driversToTry) {
        if (Get-PrinterDriver -Name $d -ErrorAction SilentlyContinue) {
            $driverName = $d
            break
        }
    }

    # Si no se encuentra por nombre exacto, buscar por patrón (wildcard)
    if (-not $driverName) {
        $found = Get-PrinterDriver -Name "*Generic*Text*", "*Genérico*texto*", "*Generic*", "*PDF*" -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($found) {
            $driverName = $found.Name
        }
    }

    if (-not $driverName) {
        throw "No se encontró ningún driver compatible (Generic / Text Only o Microsoft Print to PDF)."
    }

    # 2. Crear Puerto TCP/IP si no existe
    if (-not (Get-PrinterPort -Name $portName -ErrorAction SilentlyContinue)) {
        # Creamos el puerto TCP/IP estándar para loopback
        # Se omite -SNMP 0 por problemas de compatibilidad en algunas versiones de Windows
        Add-PrinterPort -Name $portName -PrinterHostAddress 127.0.0.1 -PortNumber 9100 | Out-Null
    }

    # 3. Crear Impresora si no existe
    if (-not (Get-Printer -Name $printerName -ErrorAction SilentlyContinue)) {
        Add-Printer -Name $printerName -DriverName $driverName -PortName $portName | Out-Null
    }

    # 4. Configuración post-instalación
    # Deshabilitamos la publicación en el directorio para evitar ruidos en red
    Set-Printer -Name $printerName -Published $false | Out-Null

    Write-Output "OK: $printerName (driver: $driverName)"
} catch {
    $msg = $_.Exception.Message
    Write-Error "Fallo en configuración de impresora: $msg"
    exit 1
}
"#;

    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .output()
        .map_err(|e| format!("No se pudo ejecutar PowerShell: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        Err(format!(
            "Fallo instalacion de impresora.\nSTDOUT:\n{stdout}\nSTDERR:\n{stderr}"
        ))
    }
}

#[cfg(windows)]
pub fn uninstall_printer() -> Result<(), String> {
    let script = r#"
$ErrorActionPreference = 'SilentlyContinue'
$portName = 'ESCPosViewer9100'
$printerName = 'ESCPos Viewer (TCP 9100)'

try {
    Get-Printer -Name $printerName | Remove-Printer | Out-Null
    Get-PrinterPort -Name $portName | Remove-PrinterPort | Out-Null
    Write-Output "OK: removed (if existed)"
} catch {
    # Ignoramos fallos en desinstalación (ej. ya no existe) pero reportamos éxito
    Write-Output "OK: cleanup attempted"
}
"#;

    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .output()
        .map_err(|e| format!("No se pudo ejecutar PowerShell: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        Err(format!(
            "Fallo desinstalacion de impresora.\nSTDOUT:\n{stdout}\nSTDERR:\n{stderr}"
        ))
    }
}

#[cfg(windows)]
pub fn set_printer_offline(offline: bool) -> Result<(), String> {
    let status_val = if offline { "$true" } else { "$false" };
    
    // Usamos Start-Process con WindowStyle Hidden para evitar CUALQUIER ventana o parpadeo.
    // El script se pasa como un bloque de comando escapado para PowerShell.
    let script = format!(
        r#"$p = Get-CimInstance Win32_Printer | Where-Object Name -eq 'ESCPos Viewer (TCP 9100)'; if ($p) {{ $p.WorkOffline = {}; Set-CimInstance -InputObject $p }}"#,
        status_val
    );

    let ps_command = format!(
        "Start-Process powershell -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', \"{}\" -WindowStyle Hidden",
        script.replace("\"", "'")
    );

    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let _ = Command::new("powershell")
        .args(["-NoProfile", "-Command", &ps_command])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("No se pudo lanzar PowerShell: {e}"))?;

    Ok(())
}

#[cfg(not(windows))]
pub fn install_printer() -> Result<(), String> {
    Err("Instalacion de impresora solo soportada en Windows".to_string())
}

#[cfg(not(windows))]
pub fn uninstall_printer() -> Result<(), String> {
    Err("Desinstalacion de impresora solo soportada en Windows".to_string())
}

#[cfg(not(windows))]
pub fn set_printer_offline(_offline: bool) -> Result<(), String> {
    Ok(())
}