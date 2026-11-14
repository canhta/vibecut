# Wake

`Wake` is the one way a worker thread asks the UI loop to repaint. The application owns a `Wake`, sets its hook to an egui repaint request and `bind`s it on the UI thread. `wake::spawn` and `wake::Builder` start threads that inherit that binding and wake it when they end; a long-lived worker calls `wake::notify` after each result it publishes. An unbound thread (the CLI, a unit test) notifies nobody, so the binding is per owner and parallel tests do not wake each other. The hook may run on any thread and must not block.
