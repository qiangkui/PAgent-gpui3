param([string]$Json = '{}')

$ErrorActionPreference = 'Stop'
try {
    [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
} catch {}

# Match the Python bridge: physical pixels, so element rectangles line up with
# SendInput click coordinates.
try {
    Add-Type -TypeDefinition @'
using System.Runtime.InteropServices;
public static class Win32DpiAwareness {
    [DllImport("shcore.dll")] public static extern int SetProcessDpiAwareness(int value);
}
'@
    [Win32DpiAwareness]::SetProcessDpiAwareness(2) | Out-Null
} catch {
    try {
        Add-Type -TypeDefinition @'
using System.Runtime.InteropServices;
public static class Win32DpiAwarenessOld {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
}
'@
        [Win32DpiAwarenessOld]::SetProcessDPIAware() | Out-Null
    } catch {}
}

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$payload = $Json | ConvertFrom-Json
function Read-Handle($value) {
    $text = [string]$value
    if ($text -match '^0x') {
        return [IntPtr]([Convert]::ToInt64($text.Substring(2), 16))
    }
    return [IntPtr]([Int64]$text)
}

function Read-Property($block, $fallback) {
    try { return (& $block) } catch { return $fallback }
}

$maxDepth = [Math]::Min(7, [Math]::Max(1, [int]$payload.depth))
$limit = [Math]::Min(500, [Math]::Max(1, [int]$payload.limit))
$includeOffscreen = $payload.include_offscreen -eq $true
$actionableOnly = $payload.actionable_only -ne $false
$nameFilter = [string]$payload.name_contains
$typeFilter = [string]$payload.type_contains
$handle = Read-Handle $payload.handle
$root = [System.Windows.Automation.AutomationElement]::FromHandle($handle)
if ($null -eq $root) {
    @{ ok = $false; error = "UIA root not found for handle $($payload.handle)" } | ConvertTo-Json -Compress
    exit 0
}

$rootCurrent = Read-Property { $root.Current } $null
$rootRect = Read-Property { $rootCurrent.BoundingRectangle } $null
$window = @{
    handle = ('0x{0:X}' -f $handle.ToInt64())
    title = [string](Read-Property { $rootCurrent.Name } '')
    class_name = [string](Read-Property { $rootCurrent.ClassName } '')
    pid = [int](Read-Property { $rootCurrent.ProcessId } 0)
    rect = $null
}
if ($null -ne $rootRect) {
    $window.rect = @{
        left = [int]$rootRect.X
        top = [int]$rootRect.Y
        width = [int]$rootRect.Width
        height = [int]$rootRect.Height
    }
}

$elements = New-Object System.Collections.ArrayList
$interactiveTypes = @(
    'Button', 'Edit', 'Text', 'Document', 'CheckBox', 'RadioButton', 'ComboBox',
    'MenuItem', 'Hyperlink', 'TabItem', 'Slider', 'ListItem', 'TreeItem',
    'DataItem', 'Cell', 'SplitButton', 'Spinner', 'ScrollBar'
)
$patternIds = @{
    Invoke = [System.Windows.Automation.InvokePattern]::Pattern
    Value = [System.Windows.Automation.ValuePattern]::Pattern
    Toggle = [System.Windows.Automation.TogglePattern]::Pattern
    SelectionItem = [System.Windows.Automation.SelectionItemPattern]::Pattern
    ExpandCollapse = [System.Windows.Automation.ExpandCollapsePattern]::Pattern
    Scroll = [System.Windows.Automation.ScrollPattern]::Pattern
    RangeValue = [System.Windows.Automation.RangeValuePattern]::Pattern
    Text = [System.Windows.Automation.TextPattern]::Pattern
}

function Walk-Child($element, [int]$depth) {
    if ($elements.Count -ge $limit) { return }
    $current = Read-Property { $element.Current } $null
    if ($null -eq $current) { return }

    $name = [string](Read-Property { $current.Name } '')
    $className = [string](Read-Property { $current.ClassName } '')
    $controlType = [string](Read-Property { $current.ControlType.ProgrammaticName } '')
    if ($controlType.StartsWith('ControlType.')) {
        $controlType = $controlType.Substring('ControlType.'.Length)
    }
    $automationId = [string](Read-Property { $current.AutomationId } '')
    $isPassword = Read-Property { $current.IsPassword } $false
    $enabled = Read-Property { $current.IsEnabled } $false
    $offscreen = Read-Property { $current.IsOffscreen } $false
    $focusable = Read-Property { $current.IsKeyboardFocusable } $false
    $focused = Read-Property { $current.HasKeyboardFocus } $false
    $rect = Read-Property { $current.BoundingRectangle } $null

    $patterns = New-Object System.Collections.ArrayList
    foreach ($patternName in $patternIds.Keys) {
        try {
            $found = $null
            if ($current.TryGetCurrentPattern($patternIds[$patternName], [ref]$found)) {
                [void]$patterns.Add($patternName)
            }
        } catch {}
    }
    $actionable = $patterns.Count -gt 0 -or $interactiveTypes -contains $controlType -or $name -ne ''
    $visibleByFilter = $includeOffscreen -or (-not $offscreen)
    $nameMatch = $nameFilter -eq '' -or $name.ToLower().Contains($nameFilter.ToLower())
    $typeMatch = $typeFilter -eq '' -or $controlType.ToLower().Contains($typeFilter.ToLower())

    if ($visibleByFilter -and $nameMatch -and $typeMatch -and ($actionable -or -not $actionableOnly)) {
        $rectValue = $null
        $center = $null
        if ($null -ne $rect -and $rect.Width -gt 0 -and $rect.Height -gt 0) {
            $rectValue = @{
                left = [int]$rect.X
                top = [int]$rect.Y
                width = [int]$rect.Width
                height = [int]$rect.Height
            }
            $center = @{
                x = [int]($rect.X + $rect.Width / 2)
                y = [int]($rect.Y + $rect.Height / 2)
            }
        }
        [void]$elements.Add([pscustomobject]@{
            index = $elements.Count + 1
            depth = $depth
            name = $name
            control_type = $controlType
            automation_id = $automationId
            class_name = $className
            rect = $rectValue
            center = $center
            enabled = [bool]$enabled
            offscreen = [bool]$offscreen
            password = [bool]$isPassword
            keyboard_focusable = [bool]$focusable
            focused = [bool]$focused
            actionable = [bool]$actionable
            patterns = @($patterns)
        })
    }

    if ($depth -lt $maxDepth -and $elements.Count -lt $limit) {
        $walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
        try {
            $child = $walker.GetFirstChild($element)
            while ($null -ne $child -and $elements.Count -lt $limit) {
                Walk-Child $child ($depth + 1)
                $child = $walker.GetNextSibling($child)
            }
        } catch {}
    }
}

try {
    $walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
    $child = $walker.GetFirstChild($root)
    while ($null -ne $child -and $elements.Count -lt $limit) {
        Walk-Child $child 1
        $child = $walker.GetNextSibling($child)
    }
} catch {
    @{ ok = $false; error = "UIA tree failed: $($_.Exception.Message)" } | ConvertTo-Json -Compress
    exit 0
}

$result = @{
    ok = $true
    action = 'uia'
    window = $window
    depth = $maxDepth
    count = $elements.Count
    truncated = ($elements.Count -ge $limit)
    elements = @($elements)
}
$result | ConvertTo-Json -Depth 8 -Compress
