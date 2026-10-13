@echo off
setlocal
rem Usage: build-moonshine-dll.cmd [models/rewrite root]
rem Run from any directory with Visual Studio C++ build tools installed.
if "%~1"=="" (set "BUNDLE_ROOT=%~dp0..\..\models\rewrite") else (set "BUNDLE_ROOT=%~1")
for %%I in ("%BUNDLE_ROOT%") do set "BUNDLE_ROOT=%%~fI"
set "RUNTIME_LIB=%BUNDLE_ROOT%\moonshine\runtime\win-x64\lib"
if not exist "%RUNTIME_LIB%\moonshine.lib" (
  echo Missing official Moonshine static library: "%RUNTIME_LIB%\moonshine.lib"
  exit /b 1
)
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%VSWHERE%" (
  echo Install Visual Studio C++ build tools first.
  exit /b 1
)
for /f "usebackq tokens=*" %%I in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VS_INSTALL=%%I"
if not defined VS_INSTALL exit /b 1
set "PATH=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer;%PATH%"
call "%VS_INSTALL%\Common7\Tools\VsDevCmd.bat" -arch=x64 -host_arch=x64 >nul
if errorlevel 1 exit /b %errorlevel%
link /NOLOGO /DLL /MACHINE:X64 /OPT:REF /OPT:ICF /OUT:"%RUNTIME_LIB%\moonshine.dll" /IMPLIB:"%RUNTIME_LIB%\moonshine-import.lib" /EXPORT:moonshine_get_version /EXPORT:moonshine_error_to_string /EXPORT:moonshine_load_transcriber_from_files /EXPORT:moonshine_free_transcriber /EXPORT:moonshine_create_stream /EXPORT:moonshine_free_stream /EXPORT:moonshine_start_stream /EXPORT:moonshine_stop_stream /EXPORT:moonshine_transcribe_add_audio_to_stream /EXPORT:moonshine_transcribe_stream "%RUNTIME_LIB%\moonshine.lib" "%RUNTIME_LIB%\moonshine-utils.lib" "%RUNTIME_LIB%\bin-tokenizer.lib" "%RUNTIME_LIB%\ort-utils.lib" "%RUNTIME_LIB%\onnxruntime.lib" bcrypt.lib ws2_32.lib ole32.lib shell32.lib advapi32.lib user32.lib
exit /b %errorlevel%
