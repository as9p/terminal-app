# Written by the terminal app, and run after the user's profile. Wraps the prompt function so
# each prompt also reports the current folder to the app, as
# ESC ] 7 ; file://<host>/<percent-encoded path> BEL, with a path such as /C:/Users/x.

# Only once, so loading this twice cannot make the prompt call itself.
if (-not (Test-Path variable:global:__TaUserPrompt)) {
    $global:__TaUserPrompt = $function:prompt
}

function global:prompt {
    # Run first, so it still sees $? from the last command.
    if ($global:__TaUserPrompt) {
        $shown = & $global:__TaUserPrompt
    } else {
        $shown = "PS $($executionContext.SessionState.Path.CurrentLocation)> "
    }
    $location = $executionContext.SessionState.Path.CurrentLocation
    if ($location.Provider.Name -eq 'FileSystem') {
        $path = '/' + ($location.ProviderPath -replace '\\', '/')
        $encoded = New-Object System.Text.StringBuilder
        foreach ($byte in [System.Text.Encoding]::UTF8.GetBytes($path)) {
            $char = [char]$byte
            if ($byte -lt 128 -and $char -match '[A-Za-z0-9/._~:-]') {
                [void]$encoded.Append($char)
            } else {
                [void]$encoded.Append(('%{0:X2}' -f $byte))
            }
        }
        [Console]::Write("$([char]27)]7;file://$env:COMPUTERNAME$encoded$([char]7)")
    }
    $shown
}
