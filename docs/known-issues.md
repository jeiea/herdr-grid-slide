# Known issues

## Quick repeated key actions can target a stale pane

Herdr's shell client sends the focus from its own view with every key action,
and the server focuses that pane before the plugin runs
([client](https://github.com/herdrdev/herdr/blob/2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01/src/client/shell/actions.rs#L244-L250),
[server](https://github.com/herdrdev/herdr/blob/2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01/src/app/custom_commands.rs#L149)).
If a press arrives before the view has caught up with the focus the previous
action set, it runs against the old pane, so a `new-pane` can land after the
wrong pane. Reproduced with Herdr 0.9.3 and 0.9.5 without this plugin; not yet
reported upstream.
