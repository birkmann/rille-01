import QtQuick
import rille.ui

// All text in the app: the UI font, readable size, tabular figures.
Text {
    color: Theme.text
    font.family: Theme.fontFamily
    font.pixelSize: Theme.fontNormal
    font.features: { "tnum": 1 }
    verticalAlignment: Text.AlignVCenter
}
