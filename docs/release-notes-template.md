## Descargas

| Sistema | Archivo |
|---|---|
| Windows 10/11 (64 bits) | `Temiktli-<versión>-windows-x64-setup.exe` |
| macOS Apple Silicon (M1 o posterior) | `Temiktli-<versión>-macos-arm64.dmg` |
| macOS Intel | `Temiktli-<versión>-macos-x64.dmg` |
| Linux (Debian, Ubuntu y derivadas) | `Temiktli-<versión>-linux-x64.deb` |
| Linux (cualquier distribución, si está disponible) | `Temiktli-<versión>-linux-x64.AppImage` |

Estos instaladores **no están firmados**, así que tu sistema te mostrará un aviso la primera vez. No significa que sean peligrosos: el código es abierto y puedes revisarlo. Cómo continuar:

### Windows
SmartScreen dirá «Windows protegió su PC». Pulsa **Más información** y luego **Ejecutar de todas formas**. Windows 11 ya incluye WebView2; en Windows 10 sin él, instala el [runtime de WebView2](https://developer.microsoft.com/microsoft-edge/webview2/).

### macOS
Abre el `.dmg` y arrastra Temiktli a **Aplicaciones**. La primera vez haz **clic derecho → Abrir** sobre la aplicación. Si macOS dice que está «dañada» o no puede verificarla, ejecuta en la Terminal:

```
xattr -cr /Applications/Temiktli.app
```

### Linux
- `.deb`: `sudo apt install ./Temiktli-<versión>-linux-x64.deb` (instala también WebKitGTK).
- AppImage: `chmod +x Temiktli-<versión>-linux-x64.AppImage` y ejecútalo.

### Verificar la descarga
Con `SHA256SUMS.txt` en la misma carpeta: `sha256sum -c SHA256SUMS.txt --ignore-missing` (Linux/macOS: `shasum -a 256 -c`).

### Tus datos
Los snippets y la configuración se guardan fuera de la aplicación y **se conservan** al actualizar o desinstalar. Para actualizar, instala la versión nueva encima.
