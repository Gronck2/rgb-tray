# RGB Tray

[English](README.md) · [Русский](README.ru.md) · [简体中文](README.zh-CN.md) · **Español** · [Deutsch](README.de.md) · [Français](README.fr.md) · [Português (Brasil)](README.pt-BR.md) · [日本語](README.ja.md)

Una pequeña aplicación de la bandeja de Windows que controla la iluminación RGB de teclados, ratones y luces directamente por HID (y las bombillas Tuya por la red local), sin software del fabricante (G HUB, Synapse, Armoury Crate, SignalRGB) ejecutándose en segundo plano.

**¿Tu dispositivo no está en la lista? Añádelo tú mismo.** Abre este repositorio en [Claude Code](https://claude.com/claude-code) y escribe `/add-device`: Claude encuentra el protocolo, escribe un perfil o un controlador y lo prueba contigo, sin necesidad de saber Rust ni HID ([cómo funciona](#añadir-tu-dispositivo-con-claude)).

<p>
  <img src="docs/screenshots/panel-dark.png" width="300" alt="Panel de la bandeja">
  <img src="docs/screenshots/editor-dark.png" width="520" alt="Editor por tecla">
</p>

## Dispositivos compatibles

| Dispositivo | Conexión | Qué funciona |
|---|---|---|
| Logitech G915 TKL | Receptor LIGHTSPEED | Estático, respiración, espectro, apagado; colores por tecla; batería |
| Razer Naga V2 Pro | Dongle HyperSpeed / base | Zonas separadas del logotipo y la rueda de desplazamiento; efectos; batería |
| Razer Mouse Dock Pro | USB | Estático, respiración, espectro, apagado |
| Unos 90 ratones, teclados y accesorios Razer más (Basilisk, DeathAdder, Viper, Cobra, BlackWidow, Huntsman, Ornata, Firefly, Goliathus, Base Station…), a partir de los datos de OpenRazer: [lista](profiles) | USB / dongle | Estático, respiración, espectro, apagado; zonas de los ratones; batería. **Sin probar**: la tarjeta lo indica, y cualquier informe es bienvenido. Por ahora, los teclados inalámbricos HyperSpeed solo funcionan por cable. |
| Teclado de portátil ASUS (Aura) | Integrado | Estático, respiración, espectro, apagado; zonas del teclado y luces del chasis mediante LampArray |
| Geekboard MacroPad v2 (QMK/VIA) | USB | Estático, respiración, espectro, apagado |
| Otros teclados QMK con VIA y RGB (Keychron, teclados personalizados…) | USB | Estático, respiración, espectro, apagado, brillo (la respiración y el espectro los dibuja la aplicación) |
| Dispositivos con el estándar Iluminación dinámica de Windows (HID LampArray) | USB / integrado | Estático, respiración, espectro, apagado, zonas del propio dispositivo (la respiración y el espectro los dibuja la aplicación) |
| Bombillas Wi-Fi Tuya (Smart Life), p. ej. LINUS A60 | Wi-Fi, red local | Color, blanco (un color casi blanco usa los LED blancos y su tono), respiración, espectro, apagado; no se apaga junto con la pantalla. Requiere la clave local de la bombilla, ver más abajo |

Otros modelos que usen los mismos protocolos pueden funcionar, pero no se han probado.

### Bombillas Tuya

RGB Tray controla una bombilla Tuya por la red local (protocolo 3.3) con la *clave local* de la bombilla, que solo entrega la nube de Tuya. Obtenla una vez, por ejemplo con [tinytuya](https://github.com/jasonacox/tinytuya) (`python -m tinytuya wizard`) o con el inicio de sesión por QR de Smart Life de `tuya-device-sharing-sdk`. Después, añade las bombillas a `%APPDATA%\rgbtray\tuya.toml`:

```toml
[[device]]
name = "Lámpara de escritorio"
id = "bf1234567890abcdef12"
local_key = "0123456789abcdef"
```

La aplicación encuentra la bombilla en la red local por sí sola y recuerda su dirección (`local_ip`). La bombilla debe estar en **Smart Life** o **Tuya Smart**: las cuentas de las copias con marca propia de la aplicación no pueden entregar claves. Una bombilla admite una sola conexión local a la vez: mientras la aplicación Smart Life esté abierta en un teléfono, RGB Tray no puede comunicarse con ella.

### Solicitar un dispositivo nuevo

Haz clic derecho en el icono de la bandeja → **Request device support…** (solicitar compatibilidad con un dispositivo). RGB Tray enumera tus dispositivos HID, marca los que pueden tener iluminación controlable, copia el informe al portapapeles y abre el [formulario de solicitud](../../issues/new?template=device-request.yml). Indica el modelo y pega el informe. Los números de serie del informe aparecen acortados.

Un dispositivo Razer o QMK/VIA, o uno con un protocolo sencillo, a menudo se puede añadir de inmediato, sin esperar a una nueva versión: basta con describirlo en un pequeño archivo de perfil ([docs/profiles.md](docs/profiles.md)).

### Añadir tu dispositivo con Claude

Con [Claude Code](https://claude.com/claude-code) puedes hacer funcionar un dispositivo por tu cuenta, sin saber Rust ni HID. Clona este repositorio, abre Claude Code en él y escribe `/add-device`. Claude lee el informe del dispositivo, busca el protocolo (OpenRGB, OpenRazer, una captura USB de la aplicación del fabricante), escribe un perfil o un controlador y lo prueba contigo: como no puede ver las luces, después de cada paso te pregunta qué hizo el dispositivo. Los comandos que podrían dañar un dispositivo (actualizaciones de firmware, guardar en la memoria flash) están prohibidos. Cuando funcione, Claude puede enviarlo como pull request para que quede integrado para todos.

## Características

La interfaz de la aplicación está disponible solo en inglés y ruso.

- Un único panel para todos los dispositivos: color, efecto y brillo, para todos a la vez o para cada dispositivo.
- Brillo global y, además, un brillo por dispositivo.
- Velocidad de la respiración y del espectro, global y por dispositivo.
- Efectos de firmware de un teclado en su editor de teclas, junto a los preajustes: onda y ondulación en el Logitech G915. Al pintar una tecla vuelven los colores por tecla.
- Editor por tecla para el teclado Logitech: pincel, cuentagotas, arcoíris, degradado de dos colores, preajustes WASD y de solo letras, resaltado de una palabra, esquemas guardados, distribuciones ANSI/ISO.
- Preajustes con atajos de teclado globales (incluidas F13–F24 con Ctrl/Alt/Shift/Win).
- Un atajo global por dispositivo que lo apaga y lo vuelve a encender tal como estaba (el botón de teclado tras el nombre en su tarjeta: F13–F24 o cualquier combinación), por ejemplo para una lámpara de la habitación. Los preajustes no se ven afectados.
- Varios dispositivos idénticos con ajustes independientes; los dispositivos se reconocen por su número de serie.
- Los dispositivos desconectados se guardan en un archivo con sus ajustes y vuelven al reconectarlos.
- Estado "No link" (sin conexión) cuando un dispositivo inalámbrico está en reposo o apagado; los colores se vuelven a aplicar cuando despierta.
- Vuelve a aplicar la iluminación tras la suspensión (incluido Modern Standby) y al conectar dispositivos.
- Actualizaciones: una vez al día la aplicación consulta en GitHub la última versión; si hay una más reciente, aparece una tarjeta en el panel que la instala con un clic. La comprobación se puede desactivar en el menú de la bandeja.
- Sigue a la pantalla: la iluminación se atenúa cuando Windows atenúa la pantalla y se apaga mientras la pantalla está apagada (inactividad, bloqueo, suspensión). Se puede desactivar en el menú de la bandeja.
- Avisos de batería baja al 10 % y al 5 %.
- El icono de la bandeja muestra el color actual.
- Modo fijado: el panel permanece abierto por encima de las demás ventanas y se puede arrastrar por su encabezado.
- Exportación e importación de todos los ajustes a un archivo. Los preajustes se pueden compartir de uno en uno: clic derecho en un preajuste → Save to a file… (guardar en un archivo), y quien lo recibe lo carga con Load settings or a preset… (cargar ajustes o un preajuste) en el menú de la bandeja; se aplica a sus dispositivos de los mismos modelos (a todos, si tiene varios), y la aplicación indica qué modelos del archivo no tiene.
- Interfaz en inglés y ruso: en ruso cuando Windows o una de sus distribuciones de teclado está en ruso, en inglés en los demás casos; se puede cambiar en el menú de la bandeja.
- Tema claro y oscuro según Windows.

## Instalación

1. Descarga `RGBTray-Setup-<version>.exe` desde [Releases](../../releases).
2. Ejecútalo. La aplicación se instala sola en `%LOCALAPPDATA%\Programs\RGB Tray` para el usuario actual, sin derechos de administrador, y se inicia.
3. Se puede desinstalar desde Configuración de Windows → Aplicaciones.

El instalador aún no está firmado digitalmente, por lo que Windows SmartScreen puede mostrar una advertencia (Más información → Ejecutar de todas formas). Los ajustes se guardan en `%APPDATA%\rgbtray` y se conservan tras actualizaciones y reinstalaciones. La carpeta `backups` que hay allí guarda copias de los ajustes de los últimos 14 días y de antes de cada actualización; si el archivo de ajustes se daña, la aplicación toma por sí sola la última copia que funcione.

Cierra el software de iluminación del fabricante antes de usar RGB Tray: dos programas que escriben en el mismo dispositivo se estorban entre sí. En Windows 11, el servicio "Logitech LampArray" (Iluminación dinámica) hace que el G915 ignore los efectos; la aplicación avisa de ello y ofrece desactivarlo.

## Línea de comandos

| Comando | Qué hace |
|---|---|
| `rgbtray --diagnose [file.txt]` | Escribe el informe de dispositivos: versiones de la aplicación y de Windows, dispositivos compatibles y todos los dispositivos HID agrupados con sus interfaces, primero los que probablemente controlan la iluminación. Sin nombre de archivo, se guarda en `%APPDATA%\rgbtray\rgbtray-diagnose.txt` y se abre en el Bloc de notas. También está disponible en el menú de la bandeja. |
| `rgbtray --hid-send <out.txt> <vid:pid[:page[:usage]]> <feature|output|get> "<hex>"…` | Envía informes HID sin procesar y añade las respuestas al archivo, para descifrar el protocolo de un dispositivo ([docs/profiles.md](docs/profiles.md#hid-any-device-with-a-simple-protocol)). |
| `rgbtray --profile-check <out.txt> <profile.toml>` | Lee un archivo de perfil sin tocar ningún dispositivo y escribe qué entiende la aplicación de cada modelo, o qué tiene de incorrecto. |
| `rgbtray --profile-test <out.txt> <profile.toml>` | Reproduce colores y efectos en los dispositivos de un perfil `[hid]` que aún no está instalado y registra cada paso. |
| `rgbtray --show` | Se inicia con el panel abierto y fijado. |
| `rgbtray --uninstall` | Desinstala la aplicación (los ajustes se conservan). |

## Compilar desde el código fuente

Requiere Windows 10/11 y [Rust](https://rustup.rs) estable, con la toolchain `x86_64-pc-windows-gnu` o `-msvc`. Después de instalar rustup, abre una terminal nueva para que `cargo` esté en el PATH.

```powershell
cargo build --release            # target\release\rgbtray.exe
.\scripts\release.ps1            # installer in dist\RGBTray-Setup-<version>.exe
.\scripts\release.ps1 -Install   # build and install/update silently
```

El instalador es el mismo ejecutable: si se llama `RGBTray-Setup-*.exe` (o se inicia con `--install`), se instala a sí mismo.

## Contribuir

Los informes de dispositivos, las pruebas en tu hardware, los informes de errores y los pull requests son bienvenidos. [CONTRIBUTING.md](CONTRIBUTING.md) explica cómo compilar y comprobar la aplicación, cómo añadir un dispositivo y qué necesita un pull request.

## Agradecimientos

Los protocolos de los dispositivos se aprendieron de estos proyectos; RGB Tray no contiene código suyo:

- [OpenRGB](https://gitlab.com/CalcProgrammer1/OpenRGB): iluminación de Logitech, Razer y ASUS Aura
- [OpenRazer](https://github.com/openrazer/openrazer): protocolo de Razer
- [Solaar](https://github.com/pwr-Solaar/Solaar): HID++ de Logitech
- [QMK](https://qmk.fm) y [VIA](https://www.caniusevia.com): protocolo de iluminación del MacroPad

## Licencia

Con licencia [MIT](LICENSE-MIT) o [Apache-2.0](LICENSE-APACHE), a tu elección.

Salvo que indiques expresamente lo contrario, cualquier contribución que envíes intencionadamente para su inclusión en este proyecto, tal como se define en la licencia Apache-2.0, tendrá la doble licencia indicada arriba, sin términos ni condiciones adicionales.

Sin afiliación con Logitech, Razer, ASUS, Geekboard ni Tuya. Las marcas comerciales pertenecen a sus propietarios.

Esta es una traducción; en caso de discrepancia, prevalece el README en inglés.
