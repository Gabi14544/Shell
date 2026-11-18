# Brasa Terminal 0.2

Terminal de escritorio y editor de código en Rust para Linux Mint 22.3 de 64 bits. Usa GTK 3, VTE y GtkSourceView 4. La interfaz está en español.

## Instalación en Mint

Después de clonar el repositorio, abrí una terminal en la carpeta `Shell` y ejecutá:

```bash
sudo apt install ./dist/brasa-terminal_0.2.0_amd64.deb
```

El instalador se encuentra en la carpeta `dist`. Si descargás solo el archivo `.deb`, instalalo con `sudo apt install ./brasa-terminal_0.2.0_amd64.deb` desde la carpeta donde lo guardaste.

Abrí **Brasa Terminal** desde el menú de aplicaciones o escribí `brasa`. Para abrir un archivo directamente, usá `brasa programa.py`.

## Paneles de trabajo

- **Archivos**, **Terminal** y **Editor** comparten una ventana. Arrastrá los separadores para ajustar el ancho. Los botones **Archivos** y **Editor** de la barra superior muestran u ocultan esos paneles.
- El explorador carga las carpetas al expandirlas. Usá el icono de carpeta para abrir otro proyecto, el icono de recarga para actualizar la lista y doble clic para abrir un archivo.
- **Ctrl+Mayús+T** lleva el foco a la terminal. **Ctrl+Mayús+E** lleva el foco al editor y lo muestra si estaba oculto. **Ctrl+Mayús+B** muestra u oculta el explorador.
- La terminal usa la shell de tu cuenta con una PTY real y 2.000 líneas de historial. **Ctrl+Mayús+C** y **Ctrl+Mayús+V** copian y pegan. La shell sigue activa mientras editás.
- El editor admite un archivo de texto UTF-8 de hasta 2 MiB a la vez. Tiene resaltado de sintaxis, números de línea, autoindentación y deshacer. Con el editor enfocado, **Ctrl+N**, **Ctrl+O** y **Ctrl+S** crean, abren y guardan archivos. Las extensiones actúan sobre archivos de hasta 512 KiB.
- La terminal y el editor usan **Hack**. El paquete de Mint instala la fuente como dependencia.

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
sudo apt install build-essential pkg-config libgtk-3-dev libvte-2.91-dev libgtksourceview-4-dev fonts-hack
cargo build --release --locked --jobs 2
./target/release/brasa
```

Para volver a crear el instalador, ejecutá `./packaging/package.sh .` desde la carpeta del proyecto.

## Consumo

Brasa crea el editor al empezar a editar. Inicia hasta un servidor de lenguaje a la vez, según el archivo abierto. Al ocultar el editor se detiene el servidor y el archivo queda abierto. Para ver el consumo de Brasa en tu equipo, ejecutá `ps -C brasa -o pid,rss,%cpu,args`. RSS se expresa en KiB e incluye memoria compartida. La shell y el servidor de lenguaje son procesos separados.
