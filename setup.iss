; Script generado para EscPos Viewer con instalación de Impresora Virtual
#define MyAppName "Visor ESC-POS"
; Importante: Evitar '/' en nombres usados como ruta (accesos directos/carpetas)
#define MyAppNameSafe "Visor ESC-POS"

; CI puede sobreescribir esta constante con:
;   ISCC setup.iss /DMyAppVersion=1.2.3
#ifndef MyAppVersion
#define MyAppVersion "1.6.0"
#endif

#define MyAppPublisher "escpos_viewer"
#define MyAppExeName "escpos_viewer.exe"

[Setup]
; Identificador único (Genera uno nuevo en Inno Setup -> Tools -> Generate GUID)
AppId={{D4522987-1954-4691-9689-536066236312}}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\{#MyAppNameSafe}
; CRÍTICO: Necesitamos permisos de administrador para crear puertos e impresoras
PrivilegesRequired=admin
OutputDir=installer
OutputBaseFilename=InstaladorVisorESCPOS
Compression=lzma
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\{#MyAppExeName}

[Languages]
Name: "spanish"; MessagesFile: "compiler:Languages\Spanish.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
; Asegúrate de compilar antes con: cargo build --release
Source: "target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "docs\CONFIGURAR_IMPRESORA_TCP_9100.txt"; DestDir: "{app}"; Flags: ignoreversion
; Si tienes un icono, descomenta esta línea:
; Source: "assets\icon.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; Accesos directos: usar nombre seguro para evitar que '/' se interprete como subcarpeta.
Name: "{autoprograms}\{#MyAppNameSafe}\{#MyAppNameSafe}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppNameSafe}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon
Name: "{autoprograms}\{#MyAppNameSafe}\Ayuda - Configurar impresora TCP 9100"; Filename: "{app}\CONFIGURAR_IMPRESORA_TCP_9100.txt"

[Run]
; 2. Opción para abrir el programa al finalizar
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent

[Code]
var
	RemovePrinterAsked: Boolean;
	RemovePrinterChoice: Boolean;

function TryRunPrinterSetup(Args: String): Integer;
var
  ResultCode: Integer;
begin
  // Ejecuta el exe instalado y devuelve el exit code (o -1 si no se pudo ejecutar).
  ResultCode := -1;
  if Exec(ExpandConstant('{app}\\{#MyAppExeName}'),
          Args,
          '',
          SW_HIDE,
          ewWaitUntilTerminated,
          ResultCode) then
  begin
    Result := ResultCode;
  end
  else
  begin
    Result := -1;
  end;
end;

function PrinterInstalled(): Boolean;
var
	ResultCode: Integer;
	Output: AnsiString;
	MarkerPath: String;
begin
	// Devuelve "YES" si existe la impresora
	Result := False;

	MarkerPath := ExpandConstant('{tmp}\\escpos_prn_ok.txt');
	DeleteFile(MarkerPath);

	// Verificación robusta usando un archivo marcador.
	if Exec('powershell',
					'-NoProfile -ExecutionPolicy Bypass -Command "$p=Get-Printer -Name ''ESCPos Viewer (TCP 9100)'' -ErrorAction SilentlyContinue; if ($p) { Set-Content -Path ''' + MarkerPath + ''' -Value ''YES'' }"',
					'',
					SW_HIDE,
					ewWaitUntilTerminated,
					ResultCode) then
	begin
		if LoadStringFromFile(MarkerPath, Output) then
		begin
			Result := (Trim(String(Output)) = 'YES');
		end;
	end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
	ExitCode: Integer;
	OpenResult: Integer;
begin
	if CurStep = ssPostInstall then
	begin
		// Intentar crear la impresora, pero nunca bloquear la instalación.
		WizardForm.StatusLabel.Caption := 'Configurando impresora virtual TCP/IP (9100)...';
		ExitCode := TryRunPrinterSetup('--install-printer');
		// ExitCode se ignora; luego verificamos si quedó instalada.
		WizardForm.StatusLabel.Caption := '';

		if not PrinterInstalled() then
		begin
			if MsgBox(
					 'No se pudo crear la impresora virtual "ESCPos Viewer (TCP 9100)" automáticamente.' + #13#10 +
					 'La instalación continuará igual. Puedes configurarla manualmente siguiendo la guía incluida.' + #13#10 + #13#10 +
					 '¿Abrir la guía ahora?',
					 mbInformation,
					 MB_YESNO) = IDYES then
			begin
				ShellExec('open', ExpandConstant('{app}\\CONFIGURAR_IMPRESORA_TCP_9100.txt'), '', '', SW_SHOWNORMAL, ewNoWait, OpenResult);
			end;
		end;
	end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
	ExitCode: Integer;
begin
	if (CurUninstallStep = usUninstall) and (not RemovePrinterAsked) then
	begin
		RemovePrinterAsked := True;
		RemovePrinterChoice := (MsgBox(
			'¿También quieres desinstalar la impresora virtual "ESCPos Viewer (TCP 9100)"?' + #13#10 +
			'(Recomendado: Sí, si ya no usarás el visor como impresora virtual.)',
			mbConfirmation,
			MB_YESNO) = IDYES);

		if RemovePrinterChoice then
		begin
			// Intentar borrar impresora/puerto, sin bloquear desinstalación.
			ExitCode := TryRunPrinterSetup('--uninstall-printer');
		end;
	end;
end;

// Nota:
// - La creación de la impresora usa cmdlets de PowerShell (PrintManagement) y requiere admin.
// - Si en algún equipo no existe el driver "Generic / Text Only" o faltan cmdlets, el exe devolverá exit code != 0.