# Instancias privadas (eventos)

Instancias de Minecraft para eventos: el organizador gestiona desde la web de Takumi la carpeta completa del juego, solo
los miembros invitados pueden jugarla, nadie puede añadir mods propios, los archivos del evento no quedan a mano en el
disco y, cuando el organizador publica algo nuevo, llega a todos (o les obliga a reiniciar).

Estado a **15 de noviembre de 2026**: las cuatro fases del plan están hechas y probadas (ver «Pruebas»). Lo que queda
pendiente está al final.

---

## Cómo encaja todo

| Pieza | Dónde | Qué hace |
|---|---|---|
| **Web (TakumiWeb)** | `takumi-webpage` | Roles, miembros, invitaciones, gestor de archivos, versiones firmadas, tickets, clave del almacén, clave de servidor y estado de versiones |
| **Takumi Launcher** | este repo | Descarga y verifica, guarda cifrado, prepara la carpeta al jugar y la limpia al cerrar, pide el ticket, vigila versiones nuevas mientras se juega |
| **Takumi Gate** (mod de Fabric) | `takumi-gate` | En el servidor del evento: solo deja entrar con ticket válido y expulsa a quien siga con una versión obligatoria vieja. En el cliente: entrega el ticket y muestra los avisos del launcher |

```
Organizador ──(web)──► publica versión N (manifiesto firmado Ed25519)
                                  │
Jugador ──► Launcher: descarga → verifica firma y SHA-256 → cifra en vault/ → pide ticket
                                  │
                  al pulsar Jugar: descifra en la carpeta del juego (+ marca de agua) → lanza
                                  │                      │
                     Takumi Gate (cliente) ── ticket ──► Takumi Gate (servidor) ──► /api/gate/verify
                                  │
                  al cerrar el juego: borra lo descifrado
```

---

## Qué se hizo

### Fase 1 — Instancias gestionadas desde la web

**Web**
- Roles: `editor` o superior crea instancias; `admin` gestiona todas. Dentro de cada instancia: `owner`, `manager`,
  `member`. Sin acceso, la instancia «no existe» (404).
- Miembros a mano (por nombre o email) o con **enlace de invitación** (`/i/<token>`, con usos máximos, caducidad y
  revocación; solo se guarda el hash del token).
- **Gestor de archivos** de la carpeta completa: subir carpetas arrastrando, editar configs en el navegador, mover,
  renombrar, borrar, descargar. Política por archivo: `enforced` (siempre igual a la web) o `default` (solo se copia si
  falta, p. ej. `options.txt`).
- **Borrador y publicación:** los cambios no llegan a nadie hasta publicar. Cada publicación es una versión numerada con
  manifiesto **firmado con Ed25519**, tiempo de gracia, notas y «forzar reinicio».
- Carpetas bloqueadas (por defecto `mods/`): cualquier archivo que no esté en la versión publicada se retira.

**Launcher** (`private_instances.rs`, `packages/manifest-system`)
- Sección *Instancias privadas*: unirse con un enlace, Descargar / Actualizar (obligatorio) / Jugar.
- **Antes de cada partida** sincroniza: verifica la firma con la clave pública integrada en el build, descarga lo que
  falta o cambió (SHA-256), borra lo que el organizador quitó y mueve a `quarantine/` lo que el jugador añadió en
  carpetas bloqueadas. Sin conexión no se juega; con una versión vieja tampoco.
- La instancia no se puede modificar desde el launcher (instalar, quitar, desactivar, duplicar, abrir carpeta).
- Si la instancia tiene servidor, el juego entra directamente (quick play).

### Fase 2 — Takumi Gate (control de acceso en el servidor)

**Web** (`src/lib/server/instances/gate.ts`, migración `0002_takumi_gate`)
- *Ajustes → Servidor del evento*: genera la **clave de servidor** `tgk_…` (se muestra una vez junto con el
  `config/takumi_gate.json` listo para pegar; generar otra revoca la anterior).
- `POST /api/launcher/instances/:id/ticket`: ticket ligado al usuario, la instancia, la cuenta de Minecraft y la versión
  instalada. Dura 12 h. Al expulsar a alguien se borran sus tickets.
- `POST /api/gate/verify` y `GET /api/gate/status` para el mod del servidor.

**Launcher**
- Pide el ticket al lanzar y lo pasa al juego en la variable `TAKUMI_GATE_TICKET` (no en la línea de comandos).

**Mod Takumi Gate** (`takumi-gate`, Minecraft 26.1.2, 26.2 y 26.3)
- Durante el login pide el ticket al cliente y lo comprueba con la web usando el perfil que **Minecraft** autenticó. Se
  rechaza antes de entrar al mundo a: quien no es miembro, expulsados, otra cuenta de Minecraft, quien no viene de
  Takumi Launcher, y versiones que ya no valen.
- Solo actúa en servidores dedicados con clave; un mundo individual o LAN nunca se bloquea.
- Config con `.bak` si se corrompe, `bypass_uuids` para staff, `allow_when_api_down` (por defecto: si la web no
  responde, no entra nadie).

### Fase 3 — Actualizaciones en directo

- **Web:** `GET /api/launcher/instances/:id/status?since=N` dice si hay versión nueva y si exige reiniciar (cualquier
  cambio fuera de `resourcepacks/`, o «forzar reinicio»).
- **Launcher** (`launch.rs`, `watch_live`): mientras el juego corre, consulta cada 30 s.
  - Si la versión nueva **solo cambia packs de recursos**, los descarga, los descifra en la carpeta y avisa al juego,
    que los activa y recarga sin reiniciar.
  - Si **exige reiniciar**, avisa en el launcher (banner + botón **Reiniciar y actualizar**) y en el juego (chat con
    el plazo y recordatorio cada minuto). El botón o `/takumi reiniciar` cierran el juego limpiamente; el launcher lo
    vuelve a abrir ya actualizado y reconecta al servidor. Si el juego no se cierra en 20 s, lo cierra el launcher.
- **Servidor (Takumi Gate):** consulta cada 30 s; a los jugadores con la versión vieja les muestra título y mensaje,
  recordatorios a 10, 5 y 1 min, y los **desconecta al acabar la gracia** (con gracia 0, en segundos).

### Fase 4 — Que los archivos no se puedan reutilizar fácilmente

- **Almacén cifrado (vault):** los archivos `enforced` de `mods/`, `resourcepacks/` y `shaderpacks/` solo se guardan
  cifrados (AES-256-GCM por bloques de 1 MiB, con el SHA-256 del archivo como dato asociado) en
  `instances/<id>/vault/objects/<sha256>.tkv`. Nombres = hashes: no hay `.jar` ni nombres legibles.
- **La clave no está en el PC:** la web la deriva por miembro (`/vault-key`, HMAC de la clave de firma) y el launcher la
  tiene solo en memoria. Sin sesión, o tras ser expulsado, los archivos no se pueden abrir.
- **Solo en claro mientras se juega:** se descifran justo antes de arrancar el juego y se borran al cerrarlo (y al abrir
  el launcher si se cerró de golpe). Si un archivo del vault está manipulado, falla el descifrado, se borra y se descarga
  de nuevo.
- **Carpetas bloqueadas mientras se juega:** en cuanto el juego llega al menú, Takumi Gate abre todos los jars de
  mods (así quedan en la caché de Java) y avisa al launcher (`.takumi/loaded.json`). El launcher quita entonces el
  permiso de lectura de `mods/` y `.fabric/processedMods/` a todos los usuarios: el juego sigue funcionando con los
  archivos que ya tiene abiertos, pero el Explorador, `copy` o cualquier otro programa reciben «acceso denegado», y la
  carpeta ni siquiera se puede listar. Al cerrar el juego se devuelve el permiso y se borra todo.
- **Jars extraídos por Fabric:** Fabric saca a `.fabric/processedMods/` los jars que van dentro de otros mods
  (jar-in-jar), y se quedaban allí en claro aunque el juego estuviera cerrado (en tu instancia había 97). Ahora se
  bloquean mientras se juega y se borran al cerrar.
- **Marca de agua:** cada `.jar`/`.zip` descifrado lleva en el comentario del zip «Takumi Events - copia de
  <usuario> - <instancia> - <fecha>». Si aparece publicado, se sabe de quién salió.
- **Carpeta oculta:** la carpeta de la instancia se marca como oculta + sistema (el Explorador no la muestra por
  defecto); en Linux/macOS, permisos 700. Ya no hay «Abrir carpeta».
- Las copias en claro que quedaran de la fase 1 (como las de tu instancia *TakumiServer*) se borran en la próxima
  sincronización.

---

## Cómo se usa

**Organizador**
1. En la web: *Instancias privadas → Nueva*. Sube la carpeta del juego, ajusta políticas y carpetas bloqueadas,
   **Publicar**.
2. *Miembros*: añade gente o crea un enlace de invitación.
3. Para el servidor del evento: instala Fabric + Fabric API + el jar de Takumi Gate en el servidor, súbelo también a
   `mods/` de la instancia, genera la clave en *Ajustes* y pega la config en `config/takumi_gate.json`. Usa
   `online-mode=true`.
4. Para actualizar: cambia archivos y **Publicar** con el tiempo de gracia que quieras (0 = inmediato).

**Jugador**
- *Instancias → Instancias privadas → Unirse con un enlace*, luego **Jugar**. Todo lo demás es automático.

---

## Configuración en producción

| Dónde | Qué |
|---|---|
| Servidor web | `MANIFEST_SIGNING_KEY` (firma los manifiestos y de ella sale la clave del vault), `STORAGE_DIR`, `BODY_SIZE_LIMIT=600M` |
| Repo del launcher | Secreto `TAKUMI_MANIFEST_PUBKEY` (lo da `GET /api/launcher/manifest-key`); el workflow de release lo integra |
| Servidor del evento | `config/takumi_gate.json` con la clave `tgk_…` generada en la web |

Cambiar `MANIFEST_SIGNING_KEY` obliga a sacar un launcher nuevo y hace que todos vuelvan a descargar los archivos.

---

## Pruebas realizadas (15/11/2026)

| Qué | Resultado |
|---|---|
| Web: `pnpm check` y 50 tests unitarios | ✅ |
| Web: 27 comprobaciones HTTP de la fase 1 y 26 de las fases 2–4 (tickets, verify, status, vault-key, revocar clave) contra una web temporal | ✅ |
| Launcher: 35 tests, clippy `-D warnings`, `fmt --check` | ✅ |
| `manifest-system`: cifrado en todos los tamaños, detección de manipulación y truncado, marca de agua | ✅ |
| Launcher contra la web real: sincroniza, nada en claro en disco, manipular el vault → se detecta y se re-descarga, marca de agua en el jar, todo se borra al cerrar, ticket y estado | ✅ |
| Takumi Gate: build en 26.1.2, 26.2 y 26.3, 12 tests unitarios, gametests de servidor, compat pack (Lithium + FerriteCore), test de cliente | ✅ |
| Takumi Gate **login real** (servidor dedicado + cliente + web): sin ticket → rechazado; ticket de otra cuenta → rechazado; ticket válido → entra; publicar versión obligatoria con gracia 0 → expulsado en ~10 s | ✅ |
| Takumi Gate en el juego: pack actualizado en directo se activa; aviso de actualización en el chat con botón | ✅ |
| **Partida real completa** (`e2e_private_instance`): el launcher sincroniza y lanza Fabric 26.3 con Fabric API y Takumi Gate; se bloquean `mods/` y `processedMods/` (no se pueden listar ni leer); el juego entra en el servidor Takumi Gate con el ticket del launcher (cuenta offline, comprobada por nombre) y sigue jugando sin errores (una de las pruebas, 11 minutos dentro del mundo); «Reiniciar y actualizar» cierra y vuelve a abrir el juego; al terminar no queda nada descifrado | ✅ |
| Cierre brusco (el juego sigue vivo tras caerse el launcher): en el siguiente arranque se desbloquea y se limpia todo | ✅ |

Cómo repetirlas: ver `verifying-the-launcher` (memoria), el README de `takumi-gate` (sección Desarrollo) y el test
ignorado `private_sync_against_web` en `private_instances.rs`.

**No probado aún:** el banner y el botón «Reiniciar y actualizar» dentro de la ventana real del launcher (el reinicio
en sí está probado con el juego real, pidiéndolo desde el código).

---

## Límites (conocidos y aceptados)

- **Nada que se ejecute en el PC del jugador es 100 % imposible de copiar.** El jugador es dueño de su PC: alguien
  que sepa lo que hace puede devolverse el permiso de lectura (`icacls`), volcar la memoria de Java o leer los archivos
  durante los segundos que tarda el juego en llegar al menú (antes del bloqueo). Lo que se consigue es que copiar los
  mods ya no sea arrastrar una carpeta.
- Sin Takumi Gate en la instancia no hay bloqueo mientras se juega (el launcher no sabe cuándo es seguro bloquear y lo
  avisa en la consola).
- `resourcepacks/`, `shaderpacks/` y `config/` no se bloquean: el juego los vuelve a leer al recargar packs o shaders
  y al guardar ajustes, y bloquearlos rompería texturas o shaders a mitad de partida. Los packs llevan marca de agua.
- La marca de agua es el comentario del zip: se puede quitar si alguien sabe que está.
- La carpeta oculta es disuasión: en Windows el jugador es dueño de su PC.
- Lo realmente valioso debe quedarse en el servidor (mods de servidor, plugins, datapacks).
- Takumi Gate es solo para Fabric. En servidores `online-mode=true` el ticket se comprueba contra el UUID que valida
  Mojang; en `online-mode=false` (cuentas no premium) se comprueba por nombre: sigue haciendo falta el ticket del
  launcher, pero quien consiga el ticket de otro podría entrar con ese nombre.

---

## Qué queda por hacer

1. **Publicar Takumi Gate:** crear el repo `AloneX15/TakumiGate`, primer run de la CI y release (ver skill
   `fabric-mod-github-ci`), icono definitivo y proyecto en Modrinth.
2. **Probar en la app real** el banner y «Reiniciar y actualizar», y un evento completo con 2–3 jugadores.
3. **Cargador en memoria** (para que los mods no toquen el disco ni durante el arranque): proveedor de sistema de
   archivos propio para Java o un lanzador que entregue los jars a Fabric desde memoria. Fabric abre los jars con
   `java.io`/`jar:` URLs, así que necesita una prueba de concepto seria; cerraría la ventana de los segundos de arranque.
4. **Marca de agua más robusta** dentro de las clases o recursos de cada jar (además del comentario del zip), para
   que sobreviva si alguien re-empaqueta el jar.
5. **Avisos instantáneos** por SSE o WebSocket en lugar de consultar cada 30 s.
6. Instalar Takumi Gate automáticamente en las instancias privadas (sin que el organizador tenga que subirlo).
7. Soporte de Quilt/NeoForge en Takumi Gate y, si se quiere, 1.21.1 como soporte extendido.
8. Límite de peticiones en `/api/gate/verify` por clave de servidor.
