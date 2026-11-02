# ezssh-testsrv

A small SSH server for tests and manual checks of the file pane: password
auth for one user, a shell channel that echoes (type `exit`), and the
`sftp` subsystem over a local folder (remote `/` = `--root`).

```sh
cargo run -p ezssh-testsrv -- --root /tmp/files [--port 2222] [--user ezssh] [--password ezssh] [--home /] [--delay-ms 0]
```

`--delay-ms` sleeps before every SFTP read and write, to watch progress
bars. Tests start it in process with `TestServer::start(Config, addr)`
(`ezssh-core/tests/sftp_api.rs`, `ezssh-app/src/ui_tests/sftp.rs`).

Not a security boundary: `..` stops at the root, but a symlink inside the
root that points out of it is followed. It binds to 127.0.0.1 only.
