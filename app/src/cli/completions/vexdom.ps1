# PowerShell completion for vexdom. Load it in $PROFILE with:
#   vexdom completions powershell | Out-String | Invoke-Expression
Register-ArgumentCompleter -Native -CommandName vexdom -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)
    $elements = @($commandAst.CommandElements | Where-Object { $_.Extent.StartOffset -lt $cursorPosition })
    $exe = $elements[0].Extent.Text
    $words = @($elements | Select-Object -Skip 1 | ForEach-Object { $_.Extent.Text })
    if ($wordToComplete -and $words.Count -gt 0) {
        $cword = $words.Count - 1
        $words[$cword] = $wordToComplete
    } else {
        $cword = $words.Count
    }
    $out = @(& $exe __complete $cword @words 2>$null)
    # :files and :dirs return nothing, so PowerShell falls back to path completion.
    if ($out.Count -eq 0 -or $out[0] -ne ':words') { return }
    foreach ($line in ($out | Select-Object -Skip 1)) {
        $value, $about = $line -split "`t", 2
        if (-not $about) { $about = $value }
        $text = if ($value -match '[\s''"`$]') { "'" + ($value -replace "'", "''") + "'" } else { $value }
        [System.Management.Automation.CompletionResult]::new($text, $value, 'ParameterValue', $about)
    }
}
