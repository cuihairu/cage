"""The plugin-facing context object: the four read/report capabilities.

Review §二 pins the first-version capability list at exactly four APIs —
``ctx.table()`` / ``ctx.schema()`` / ``ctx.error()/warning()`` /
``ctx.options()`` — all read-only except the controlled diagnostic writes.
The plugin never opens ``Monster.xlsx`` itself and never writes data back;
it is a pure function ``Diagnostics = f(Schema, Data, Options)``.
"""

from __future__ import annotations

from typing import Any, Dict, List, Optional

from .protocol import make_diagnostic


class RunContext:
    """Read-only view over the run's inputs plus the diagnostic sink."""

    def __init__(self, params: Dict[str, Any]) -> None:
        self.plugin: Dict[str, Any] = params.get("plugin", {})
        self._schema: Any = params.get("schema", {})
        self._tables: Dict[str, Dict[str, Any]] = {
            table["name"]: table for table in params.get("tables", [])
        }
        self._options: Any = params.get("options", {})
        self._diagnostics: List[Dict[str, Any]] = []

    # -- ctx.schema() ------------------------------------------------------
    def schema(self) -> Any:
        """The canonical Schema as JSON (the same shape the editor stores)."""
        return self._schema

    # -- ctx.table() -------------------------------------------------------
    def table_names(self) -> List[str]:
        """Names of the tables handed to this run, in declaration order."""
        return list(self._tables)

    def table(self, name: str) -> Optional[Dict[str, Any]]:
        """One normalized table: ``{"name", "primary_key", "rows"}``, or
        ``None`` when the project has no table of that name.

        Rows carry ``index`` (0-based), optional ``location`` (source
        position) and ``fields`` (name → normalized JSON value).
        """
        return self._tables.get(name)

    # -- ctx.options() -----------------------------------------------------
    def options(self) -> Any:
        """Project-configuration options for this plugin (read-only)."""
        return self._options

    # -- ctx.error()/warning() --------------------------------------------
    def error(
        self,
        message: str,
        *,
        code: Optional[str] = None,
        table: Optional[str] = None,
        row: Optional[int] = None,
        field: Optional[str] = None,
        hint: Optional[str] = None,
    ) -> None:
        """Report a hard violation."""
        self._add("error", message, code, table, row, field, hint)

    def warning(
        self,
        message: str,
        *,
        code: Optional[str] = None,
        table: Optional[str] = None,
        row: Optional[int] = None,
        field: Optional[str] = None,
        hint: Optional[str] = None,
    ) -> None:
        """Report a soft finding."""
        self._add("warning", message, code, table, row, field, hint)

    def info(
        self,
        message: str,
        *,
        code: Optional[str] = None,
        table: Optional[str] = None,
        row: Optional[int] = None,
        field: Optional[str] = None,
        hint: Optional[str] = None,
    ) -> None:
        """Report an informational notice."""
        self._add("info", message, code, table, row, field, hint)

    def diagnostics(self) -> List[Dict[str, Any]]:
        """Findings collected so far (a copy)."""
        return list(self._diagnostics)

    def _add(
        self,
        severity: str,
        message: str,
        code: Optional[str],
        table: Optional[str],
        row: Optional[int],
        field: Optional[str],
        hint: Optional[str],
    ) -> None:
        self._diagnostics.append(
            make_diagnostic(
                severity,
                message,
                code=code,
                table=table,
                row=row,
                field=field,
                hint=hint,
            )
        )
