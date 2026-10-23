# Security

Kova promises that a Kova program cannot read or write memory it does not own: no reading past the end of a
`List`, no use of a value after it was freed, no silent overflow. A way to break that promise (a program
`kova run` or `kova build` accepts and that does one of these things), or any other flaw that could harm
the people running Kova programs, is a security flaw.

**Please do not open a public issue for it.** Report it privately instead: on GitHub, open the
**Security** tab of this repository and choose **Report a vulnerability**.

You will get an answer within a week. The flaw is fixed first, then published with the fix, crediting you
if you wish.
