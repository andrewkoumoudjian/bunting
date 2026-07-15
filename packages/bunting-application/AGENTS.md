# Bunting application-service instructions

Keep this package transport-neutral and sans-I/O. It composes verified identity,
engine commands, origin transactions, recovery, projections, and FIX application
mapping without sockets, files, Worker bindings, ambient time, or mutable engine
handles. A result is externally publishable only after origin commit succeeds.
