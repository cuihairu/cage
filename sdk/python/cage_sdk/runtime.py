"""The worker main loop: one request in, one response out, then exit.

Exit-code contract: ``0`` when a response was written (even one carrying
error-severity findings — the *host* decides what findings mean), ``1`` on
any protocol or plugin failure. Process lifecycle (timeout, crash, spawn)
belongs to the host side (P2-2 failure-behaviour table).
"""

from __future__ import annotations

import sys
from typing import IO, Optional

from .context import RunContext
from .protocol import (
    ProtocolError,
    encode_response,
    parse_request,
    response_err,
    response_ok,
    validate_diagnostics,
)

#: Cap on the exception text carried in a PLUGIN_ERROR response, so a plugin
#: raising with a huge message cannot bloat the wire (400 bytes, cut at a
#: UTF-8 character boundary — same budget as the script-adapter stderr cap).
_PLUGIN_ERROR_MESSAGE_CAP = 400


class Plugin:
    """Base class for a Cage validation plugin.

    Subclasses set ``id``/``version`` (declared to the host in the response
    flow and used for the P2-4 attestation) and implement ``validate``.
    """

    id = "plugin"
    version = "0.0.0"

    def validate(self, ctx: RunContext) -> None:
        """Run the rule; report findings through ``ctx.error()/warning()``."""
        raise NotImplementedError


def truncate_utf8(text: str, cap: int = _PLUGIN_ERROR_MESSAGE_CAP) -> str:
    """Truncate ``text`` to at most ``cap`` bytes on a UTF-8 char boundary."""
    encoded = text.encode("utf-8")
    if len(encoded) <= cap:
        return text
    return encoded[:cap].decode("utf-8", errors="ignore")


def serve(
    plugin: Plugin,
    input_stream: Optional[IO[bytes]] = None,
    output_stream: Optional[IO[bytes]] = None,
) -> int:
    """Read one request, run ``plugin``, write one response.

    Reads all of ``input_stream`` (the MVP is single-shot: one request per
    process), so the host closes stdin after writing the request. Returns
    the process exit code.
    """
    if input_stream is None:
        input_stream = sys.stdin.buffer
    if output_stream is None:
        output_stream = sys.stdout.buffer

    data = input_stream.read()
    try:
        request = parse_request(data)
    except ProtocolError as error:
        # A pre-validation failure carries no request id to echo (``id`` is
        # omitted rather than guessed): the response reports the reason via
        # a stable ``code``, including the requested protocol version for
        # PROTOCOL_VERSION_MISMATCH.
        output_stream.write(encode_response(response_err("", error)))
        output_stream.flush()
        return 1

    request_id = request["id"]
    try:
        ctx = RunContext(request["params"])
        plugin.validate(ctx)
        diagnostics = validate_diagnostics(ctx.diagnostics())
    except ProtocolError as error:
        output_stream.write(encode_response(response_err(request_id, error)))
        output_stream.flush()
        return 1
    except Exception:
        detail = _format_exception()
        output_stream.write(
            encode_response(
                response_err(request_id, ProtocolError.plugin_error(detail))
            )
        )
        output_stream.flush()
        return 1

    output_stream.write(encode_response(response_ok(request_id, diagnostics)))
    output_stream.flush()
    return 0


def _format_exception() -> str:
    """One-line exception summary (type, message, raising frame), capped."""
    exc_type, exc_value, exc_tb = sys.exc_info()
    name = exc_type.__name__ if exc_type is not None else "Exception"
    message = str(exc_value) if exc_value is not None else ""
    frame = ""
    while exc_tb is not None and exc_tb.tb_next is not None:
        exc_tb = exc_tb.tb_next
    if exc_tb is not None:
        frame = f" at {exc_tb.tb_frame.f_code.co_filename}:{exc_tb.tb_lineno}"
    return truncate_utf8(f"{name}: {message}{frame}")


def run(plugin: Plugin) -> None:
    """``__main__`` entrypoint helper: serve and exit with its code."""
    sys.exit(serve(plugin))
