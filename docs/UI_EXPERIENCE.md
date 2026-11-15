# Mejoras visuales, animaciones y sonidos

Esta actualización mejora la presentación del launcher conservando su identidad oscura, el logotipo y los colores originales. Los cambios se concentran en el frontend Svelte de `apps/launcher/src`.

## Diseño visual

- Navegación lateral con iconos SVG, indicador de sección activa y acceso rápido al sonido.
- Pantalla de instancias con contador, bloque de bienvenida y una ilustración geométrica de un cubo realizada con SVG y CSS.
- Tarjetas con degradados discretos, sombras, bordes reactivos y separación entre información y acciones.
- Estado de biblioteca vacía con icono y acceso a la creación de la primera instancia.
- Pantalla de inicio de sesión con acento superior carmesí, mayor profundidad y control de sonido.
- Modales con fondo desenfocado y entrada animada.
- Tipografía, espaciado y estados de foco más definidos.
- Ajustes de distribución para la ventana mínima del launcher, de 920 × 600, y su tamaño inicial, de 1120 × 720.

Se mantienen las variables originales de color en `app.css`, entre ellas:

| Variable | Valor | Uso |
| --- | --- | --- |
| `--bg` | `#0d0d11` | Fondo principal |
| `--surface` | `#16161c` | Superficies |
| `--surface-2` | `#1e1e26` | Superficies secundarias |
| `--accent` | `#dc143c` | Acento carmesí |
| `--accent-hover` | `#f02a52` | Interacción con el acento |
| `--text` | `#ededf2` | Texto principal |

## Movimiento

Las animaciones se implementan con CSS, sin añadir una biblioteca de movimiento:

- Entrada de vistas al cambiar de sección.
- Entrada escalonada de tarjetas, con retraso limitado para bibliotecas grandes.
- Elevación de tarjetas al pasar el cursor y respuesta de los botones al pulsarlos.
- Entrada de la pantalla de inicio de sesión, los modales y las notificaciones.
- Flotación suave del cubo decorativo de la pantalla de instancias.

La curva compartida `--ease-out` está definida en `app.css`. Las animaciones de entrada duran aproximadamente entre 180 y 500 ms; la flotación decorativa tiene un ciclo de 7 segundos.

La opción **Ajustes → Experiencia de la interfaz → Reducir animaciones** desactiva las animaciones y transiciones CSS. También se respeta `prefers-reduced-motion: reduce` del sistema, aunque la opción local esté desmarcada. La política global cubre las animaciones existentes, como el indicador de carga y el progreso indeterminado.

## Sonidos y preferencias

Los efectos se generan localmente con Web Audio API. Son tonos sinusoidales breves, de unos 80 ms, con una envolvente suave y un tono diferente para botones con la clase `primary`. No necesitan archivos de audio ni descargas.

El cierre tiene un tono descendente propio, de 440 a 180 Hz. Se reproduce cuando un modal se desmonta realmente: botón de cierre, **Cancelar**, Escape, pulsación en el fondo o cierre tras completar una acción. Si el cierre se bloquea, por ejemplo durante una instalación de modpack, no suena. Los botones **Volver** de Contenido y Consola utilizan el mismo efecto mediante `data-sound="close"`. El cierre respeta el silencio y el volumen configurados y tiene prioridad sobre el límite de clics de 55 ms.

Los botones de cierre y cancelación de modales omiten el clic genérico con `data-sound="off"`; el propio modal emite el sonido al cerrarse, evitando duplicar el efecto. Al desmontar la aplicación completa se desactiva la emisión de nuevos sonidos.

El controlador escucha clics reales en botones, enlaces, casillas, opciones de radio y selectores. La activación mediante teclado también puede producir el clic del control. Se omiten los controles deshabilitados, los clics sintéticos y los elementos marcados con `data-sound="off"`; los clics separados por menos de 55 ms se limitan para evitar acumulación. El selector de silencio utiliza esa exclusión.

El contexto de audio se crea al interactuar. Si la API no está disponible o falla la reproducción, la acción del usuario continúa. Al desmontar la aplicación se elimina el listener y se cierra el contexto de audio.

| Preferencia | Valor inicial | Control |
| --- | --- | --- |
| Sonido | Activado | Pantalla de acceso, barra lateral y Ajustes |
| Volumen | 35 % | Ajustes; rango de 0 a 100 en pasos de 5 |
| Reducir animaciones | Desactivado | Ajustes; la preferencia del sistema también se aplica |

Estas preferencias se guardan automáticamente en `localStorage` con la clave `takumi-experience`, por dispositivo y origen de la interfaz. Son independientes del botón **Guardar cambios** de los ajustes de Java. Si el almacenamiento está bloqueado, siguen funcionando durante la sesión; los datos ausentes o inválidos se sustituyen por valores seguros y el volumen se limita al rango permitido.

## Estado de conexión con la web

La barra lateral y la pantalla de acceso incluyen un botón de estado. Al pulsarlo se repite la comprobación; mientras hay una petición en curso, el botón permanece deshabilitado para evitar solicitudes duplicadas.

| Estado | Significado |
| --- | --- |
| Comprobando web… | Solicitud en curso |
| Web conectada | La API respondió correctamente; muestra el tiempo total de respuesta en milisegundos |
| Web sin conexión | No se pudo completar la conexión con la web configurada |
| Error en la web | Respuesta HTTP de error, contenido incompatible con la API o fallo al ejecutar la comprobación |
| Web sin comprobar | Todavía no se ha verificado; en un navegador sin Tauri se indica que hace falta la aplicación de escritorio |

El tooltip muestra el detalle, la URL configurada y la hora de la última comprobación. El botón utiliza el verde y el rojo de estado ya presentes en la paleta, además de texto para identificar el resultado.

La comprobación se realiza al mostrar el componente, cada 60 segundos mientras la interfaz está visible y al recuperar el foco, volver a una pestaña visible o recibir eventos de cambio de conectividad. Los temporizadores y listeners se eliminan al desmontarlo.

El comando de Tauri `check_web_connection`, registrado en `src-tauri/src/main.rs` e implementado en `src-tauri/src/web_connection.rs`, consulta `GET /api/news?limit=1` con un timeout de 5 segundos y `Cache-Control: no-cache`. Utiliza la misma URL base que el resto de la integración (`session::web_url()`), verifica un estado HTTP correcto y deserializa la respuesta de noticias. Una página HTML de mantenimiento con HTTP 200 no se considera conectada. La consulta es pública, no envía credenciales y no modifica la sesión. El resultado verifica la disponibilidad de la API, no la validez de una sesión almacenada.

## Archivos implicados

Rutas relativas a `apps/launcher/src`:

| Archivo | Responsabilidad |
| --- | --- |
| `lib/experience.js` — nuevo | Store de preferencias, persistencia, síntesis de audio y ciclo de vida del listener |
| `components/Icon.svelte` — nuevo | Iconos SVG reutilizables sin dependencias externas |
| `components/SoundToggle.svelte` — nuevo | Control de silencio con estado accesible mediante `aria-pressed` |
| `App.svelte` | Inicialización y limpieza de efectos, navegación con `aria-current`, entrada de vistas y notificaciones con región viva |
| `app.css` | Acabado visual compartido, foco visible y política global de movimiento reducido |
| `components/Modal.svelte` | Desenfoque del fondo y animación de entrada |
| `views/Instances.svelte` | Bienvenida, contador, ilustración, tarjetas y distribución adaptable |
| `views/Login.svelte` | Presentación de acceso y control de sonido |
| `views/Settings.svelte` | Controles de sonido, volumen y movimiento |
| `components/NewInstanceModal.svelte`, `components/InstanceSettingsModal.svelte` | Cancelación sin duplicar el clic genérico y el sonido de cierre |
| `views/Console.svelte`, `views/Content.svelte` | Sonido de cierre en los botones Volver |
| `components/ConnectionStatus.svelte` | Botón de conexión compartido, comprobación manual y actualización periódica |

Esta actualización no añade dependencias de producción ni cambia el formato de las instancias. El indicador de conexión añade el comando de Tauri descrito arriba.

## Validación realizada

- Compilación de producción correcta con `pnpm --filter takumi-launcher-ui build`.
- Comprobaciones de interfaz con Playwright sobre Microsoft Edge, usando una API de Tauri simulada y datos ficticios: navegación, generación de osciladores de audio, silencio, volumen persistido tras recargar, reducción de movimiento local y del sistema, apertura de modales y cierre con Escape, biblioteca vacía y habilitación del formulario de acceso.
- Ausencia de errores de ejecución durante esas comprobaciones.
- Comprobación posterior del efecto de cierre: un único tono de 440 a 180 Hz al cerrar mediante **Cerrar**, **Cancelar**, Escape o el fondo del modal; el botón **Volver** de Consola utiliza el mismo tono y el cierre de modales respeta el silencio.
- Comprobaciones del indicador con la API de Tauri simulada: conexión correcta y latencia, web inaccesible, error de API, fallo del comando, reintento manual, botón deshabilitado durante la petición y recuperación al cambiar la conectividad. Presencia del botón en la pantalla de acceso y revisión visual a tamaño compacto.
- Prueba Rust superada con `cargo test -p takumi-launcher web_connection::tests -- --nocapture`: servidores locales simulados para respuesta válida, HTML con HTTP 200, JSON incompatible, HTTP 503 y conexión rechazada. No utiliza cuentas, credenciales ni la web real.
- Inspección de capturas de instancias, modal, inicio de sesión y distribución compacta; comprobación de ausencia de desbordamiento horizontal de la página.
- Revisión de espacios en blanco con `git diff --check -- apps/launcher/src`.

El script de comprobación fue temporal y no forma parte de la suite del repositorio. Estas pruebas no utilizaron cuentas ni instancias reales. Queda pendiente escuchar y comprobar el sonido dentro de la aplicación de escritorio con WebView2; la generación de audio comprobada en el navegador no sustituye esa prueba.

### Comprobación manual en escritorio

1. Desde `apps/launcher`, ejecutar `cargo tauri dev` con los requisitos de desarrollo instalados.
2. Comprobar los clics de navegación y de botones principales con una sesión de prueba.
3. Silenciar desde la barra lateral y confirmar que las acciones siguen funcionando sin sonido.
4. Cambiar el volumen en Ajustes, reiniciar el launcher y comprobar que se conserva.
5. Activar **Reducir animaciones** y comprobar vistas, modales, tarjetas e indicadores de carga. Repetir con la preferencia equivalente del sistema.
6. Revisar las vistas a 1120 × 720 y 920 × 600 y verificar que las acciones siguen siendo accesibles mediante desplazamiento y teclado.
7. Con la web configurada en ejecución, comprobar **Web conectada** y la latencia. Detener la web, pulsar el botón y comprobar **Web sin conexión**; volver a arrancarla y repetir para verificar la recuperación. Una respuesta HTTP de error o un documento HTML deben mostrar **Error en la web**.
