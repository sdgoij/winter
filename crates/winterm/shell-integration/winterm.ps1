# winterm shell integration (ADR 060).
#
# Emits the FinalTerm marks the scrollbar's ticks are built from:
#
#   OSC 133;A          the prompt is about to be drawn
#   OSC 133;B          the prompt ended; the command line starts here
#   OSC 133;C          the command line was submitted; its output starts here
#   OSC 133;D;<code>   the command finished; <code> is nonzero when it failed
#
# winterm loads this file with `powershell.exe -NoExit -File <this file>`.
# Dot-source it yourself if the shell is started some other way. Set
# WINTERM_SHELL_INTEGRATION=0 to keep the terminal from attaching it.

if ($env:WINTERM_SHELL_INTEGRATION -eq '0') { return }

$Global:__winterm_esc = [char]27
$Global:__winterm_bel = [char]7

function Global:__winterm_osc([string] $payload) {
    try {
        [Console]::Write($Global:__winterm_esc + ']' + $payload + $Global:__winterm_bel)
    } catch {
        # A closed console is not worth an error on the next prompt.
    }
}

# The prompt this shell already had, usually a profile's. Captured once, so
# sourcing this file twice does not wrap the wrapper.
if (-not $Global:__winterm_prompt) {
    $Global:__winterm_prompt = $function:prompt
}

# True between a prompt being drawn and the next one: a command line is waiting
# or running, and its exit code belongs in the `D` that the next prompt sends.
$Global:__winterm_command = $false

function Global:prompt {
    # `$?` and `$LASTEXITCODE` describe the command that just finished. Read
    # them before anything else here can touch them.
    $succeeded = $?
    $code = $Global:LASTEXITCODE

    if ($Global:__winterm_command) {
        # A failed cmdlet leaves `$LASTEXITCODE` alone, so a stale native exit
        # code may win here; the mark only needs "nonzero", and this keeps the
        # real code when there is one.
        $exit = if ($succeeded) { 0 } elseif ($code) { $code } else { 1 }
        __winterm_osc ('133;D;' + $exit)
        $Global:__winterm_command = $false
    }

    __winterm_osc '133;A'
    $text = & $Global:__winterm_prompt
    __winterm_osc '133;B'
    $Global:__winterm_command = $true
    $text
}

# `C` needs to know when the line was submitted, which only the line editor can
# say. Without PSReadLine the marks still work; there is just no Output mark.
try {
    if (Get-Module -ListAvailable -Name PSReadLine) {
        Import-Module PSReadLine -ErrorAction Stop
        Set-PSReadLineKeyHandler -Key Enter -BriefDescription 'winterm shell integration' -ScriptBlock {
            [Console]::Write($Global:__winterm_esc + ']133;C' + $Global:__winterm_bel)
            [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine()
        } | Out-Null
    }
} catch {
    # An unavailable line editor is not a reason to lose the prompt marks.
}
