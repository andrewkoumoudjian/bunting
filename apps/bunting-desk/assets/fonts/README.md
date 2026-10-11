# Bundled fonts

Both families are licensed under the SIL Open Font License 1.1; each licence
is kept beside the files. They are embedded in the `bunting-desk` binary
(`src/theme.rs`) and are not modified.

| Files | Family | Upstream | Archive SHA-256 |
|---|---|---|---|
| `Inter-Regular.ttf`, `Inter-Medium.ttf`, `Inter-SemiBold.ttf` (`extras/ttf/` in the archive) | Inter 4.1 | https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip | `9883fdd4a49d4fb66bd8177ba6625ef9a64aa45899767dde3d36aa425756b11e` |
| `SourceSerif4Display-Regular.ttf`, `SourceSerif4Display-Semibold.ttf` (`TTF/` in the archive) | Source Serif 4 Display 4.005 | https://github.com/adobe-fonts/source-serif/releases/download/4.005R/source-serif-4.005_Desktop.zip | `549fdb8f9a682bd06944298621404969f6de77c2e422ff3b8244a1dcd6a0c425` |

SF Pro is not bundled: Apple's licence allows it only on Apple platforms, so
macOS builds use the system font (`.SystemUIFont`) and other platforms use
Inter.
