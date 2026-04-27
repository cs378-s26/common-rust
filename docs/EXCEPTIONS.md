# Exceptions

Exceptions are fake on our system. They are handled like interrupts. When an exception occurs,
interrupts are disabled, and then the core switches to a dedicated interrupt stack and begins executing
the interrupt handler. This is the same routine that happens for external interrupts. Thus, exception
handlers must follow the same guidelines as interrupt handlers: they cannot block and most run in 
O(1) time. The recommended way to get around this restriction is to add a new constructor to the
`Event` enum (see EVENT.md for more details), and to simply push an event in the exception
handler. 