Peer identity tests no longer leak their client process
The harness_peer_identity tests killed the shell above the connecting client but not the client itself, so a sleeping python process kept the test stdio open and nextest reported LEAK. The tests now null the stdio of the fake session and kill the connecting client by pid before returning.
