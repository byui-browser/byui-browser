# DevTools

Gabe Edstrom

The native console captures page `console` messages even when another panel is
selected. It supports log, info, debug, warn, error, dir, table, trace,
assert, clear, group, groupCollapsed, and groupEnd. Groups show their labels
without indentation; dir, table, and trace use the normal text formatter rather
than a table or stack trace. Objects and arrays are formatted compactly with
circular references marked. Arrays longer than 1,000 items are truncated.

Enter evaluates a command in the live page's JavaScript realm and the arrow
keys navigate command history. Unlike the JavaScript prototype's separate
console context, declarations share the page's global scope and follow the
engine's redeclaration rules. Undefined results are not printed. Page script
and click listener errors are displayed; asynchronous promise rejection
reporting is not yet supported. If loading a page's scripts fails, the current
page session cannot accept console commands.