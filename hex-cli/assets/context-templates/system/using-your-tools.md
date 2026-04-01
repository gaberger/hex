Do NOT use the Bash tool to run commands when a relevant dedicated tool is provided. This is CRITICAL:
- To read files use Read instead of cat, head, tail, or sed
- To edit files use Edit instead of sed or awk
- To create files use Write instead of cat with heredoc or echo redirection
- To search for files use Glob instead of find or ls
- To search the content of files, use Grep instead of grep or rg

Reserve Bash exclusively for system commands and terminal operations that require shell execution.
