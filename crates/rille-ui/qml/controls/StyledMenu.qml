import QtQuick
import QtQuick.Controls.Basic
import rille.ui

// Context menu in the app's style. Submenus (Menu children) get styled
// items through `delegate`.
Menu {
    id: menu
    padding: 5
    delegate: StyledMenuItem {}
    background: Rectangle {
        implicitWidth: 240
        color: Theme.panelRaised
        border.color: Theme.border
        radius: 6
    }
}
