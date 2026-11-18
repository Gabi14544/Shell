# Brasa Terminal 0.1

Terminal de escritorio y editor de código en Rust para Linux Mint 22.3 de 64 bits. Usa GTK 3, VTE y GtkSourceView 4. La interfaz está en español.

## Instalación en Mint

Después de clonar el repositorio, abrí una terminal en la carpeta `Shell` y ejecutá:

```bash
sudo apt install ./dist/brasa-terminal_0.1.0_amd64.deb
```

El instalador se encuentra en la carpeta `dist`. Si descargás solo el archivo `.deb`, instalalo con `sudo apt install ./brasa-terminal_0.1.0_amd64.deb` desde la carpeta donde lo guardaste.

Abrí **Brasa Terminal** desde el menú de aplicaciones o escribí `brasa`. Para abrir un archivo directamente, usá `brasa programa.py`.

## Terminal y editor

- **Terminal** ejecuta la shell configurada en tu cuenta con una PTY real y 2.000 líneas de historial en memoria. Usá **Ctrl+Mayús+C** y **Ctrl+Mayús+V** para copiar y pegar.
- **Editor** abre un archivo a la vez. Tiene resaltado de sintaxis, números de línea, autoindentación y deshacer. **Ctrl+N**, **Ctrl+O** y **Ctrl+S** crean, abren y guardan archivos. El editor admite texto UTF-8 de hasta 2 MiB. Las extensiones actúan sobre archivos de hasta 512 KiB.
- **Ctrl+Mayús+E** cambia al editor y **Ctrl+Mayús+T** vuelve a la terminal. Al volver a la terminal se detiene el servidor de lenguaje. El archivo del editor queda abierto.

## Python

1. Abrí **Extensiones** y elegí **Instalar** junto a **Python · Jedi y Ruff**. La descarga usa PyPI y requiere `python3-venv`. Brasa instala los paquetes en un entorno propio bajo `${XDG_DATA_HOME:-$HOME/.local/share}/brasa/python-lsp`.
2. Abrí un archivo `.py` o guardá uno nuevo con esa extensión. Probá escribir `import os` y luego `os.pa` en otra línea. Las sugerencias aparecen tras una pausa breve. También podés pedirlas con **Ctrl+Espacio**. Usá las flechas y **Enter** o **Tab** para aceptar.
3. Si Ruff detecta un problema, **Avisos** muestra los detalles. Colocá el cursor sobre la línea y usá **Ctrl+.** o **Correcciones**. Por ejemplo, Ruff puede proponer quitar un `import` sin usar. Elegí la sugerencia y guardá el archivo.

## Otros lenguajes

- Para **Rust**, instalá `rust-analyzer` en la toolchain que usás y activalo desde **Extensiones**. Con Rustup se instala mediante `rustup component add rust-analyzer`.
- Para **C y C++**, instalá `clangd` con `sudo apt install clangd` y activalo desde el panel. Brasa lo inicia con un solo hilo de trabajo y la indexación en segundo plano deshabilitada.
- Para otros lenguajes, instalá su servidor y elegí **Agregar servidor LSP…**. Indicá el ID de lenguaje que el servidor espera, las extensiones de archivo y el comando que lo inicia por entrada y salida estándar. Podés indicar una ruta absoluta si el comando no está en el PATH. Por ejemplo, para Go usá nombre `Go · gopls`, ID `go`, extensión `go` y comando `gopls`.
- Las extensiones usan el protocolo LSP por stdio. Los paquetes `.vsix` pertenecen a VS Code y requieren su sistema de extensiones. Brasa acepta completados en texto y correcciones de un solo archivo que el servidor entrega como ediciones. La disponibilidad de sugerencias y correcciones depende del servidor y del proyecto.

## Compilar desde el código fuente

Con Rust estable instalado por Rustup y activo:

```bash
sudo apt install build-essential pkg-config libgtk-3-dev libvte-2.91-dev libgtksourceview-4-dev
cargo build --release --locked --jobs 2
./target/release/brasa
```

Para volver a crear el instalador, ejecutá `./packaging/package.sh .` desde la carpeta del proyecto.

## Consumo

Brasa crea el editor al abrirlo por primera vez. Inicia hasta un servidor de lenguaje a la vez, según el archivo abierto. Para ver el consumo de Brasa en tu equipo, ejecutá `ps -C brasa -o pid,rss,%cpu,args`. RSS se expresa en KiB e incluye memoria compartida. La shell y el servidor de lenguaje son procesos separados.
