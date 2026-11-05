# Seguridad

## Reportar una vulnerabilidad

Si encuentras un fallo de seguridad en RusIO, **no abras un issue público**. Usa el reporte privado de GitHub
(pestaña *Security* → *Report a vulnerability*) o escribe a **soy@fiambre.dev**.

Incluye, si puedes: la versión de RusIO, el sistema operativo, los pasos para reproducirlo y el impacto que
imaginas. Intentaré responder en pocos días y avisarte cuando haya una corrección.

## Qué se considera en alcance

- Contraseñas, claves o URI con credenciales que lleguen a disco, a logs o a la red sin querer.
- Ejecución de código o lectura/escritura de archivos fuera de lo que el usuario ordenó (por ejemplo,
  en SFTP o en el editor remoto).
- Fallos en la validación de túneles, saltos SSH o certificados TLS.

## Actualizaciones firmadas

El actualizador solo ejecuta un instalador descargado de `github.com/Fiambre/RusIO/releases` cuya firma Ed25519
verifica con la clave pública incluida en la app. La clave privada es el secreto `UPDATE_SIGNING_KEY` del
repositorio. Si sospechas que se filtró, repórtalo por el canal privado de arriba: habrá que rotar la clave y publicar una versión
nueva con la clave pública cambiada, instalada a mano.

## Cómo se tratan los secretos

Las contraseñas se guardan en el almacén de credenciales del sistema (servicio `rusio`) y no en
`connections.json`. Si ves una contraseña en texto plano en cualquier archivo creado por RusIO, es un bug de
seguridad: repórtalo.

## Versiones con soporte

Solo se corrigen fallos en la última versión publicada.
