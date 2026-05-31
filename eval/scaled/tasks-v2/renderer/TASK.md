Add an `{{exec:CMD}}` template directive: when `Engine::expand` is asked for a token of the form
`exec:CMD`, run `CMD` with the system shell (`sh -c`) and expand the token to the command's stdout
(trimmed). Other tokens keep their current snippet-cache behaviour.
