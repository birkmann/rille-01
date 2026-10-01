import QtQuick
import rille.ui

// The lowercase wordmark: Geist 500, tracked −5 %. Never capitalised.
UiText {
    property real size: 18
    text: "rille"
    color: Theme.brandPaper
    font.family: Theme.fontFamily
    font.pixelSize: size
    font.weight: Font.Medium
    font.letterSpacing: -0.05 * size
}
