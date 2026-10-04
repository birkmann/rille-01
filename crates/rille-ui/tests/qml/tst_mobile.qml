// Mobile layout: the tab bar and when the layout switches.
// Run with scripts/qmltest.sh.
import QtQuick
import QtTest
import rille.ui

Item {
    id: root
    width: 390
    height: 200
    property int picked: -1

    MobileTabBar {
        id: bar
        width: root.width
        height: Theme.tabBarHeight
        onSelected: i => {
            root.picked = i
            bar.current = i
        }
    }

    TestCase {
        name: "MobileLayout"
        when: windowShown

        function tab(i) {
            return findChild(bar, "tab" + i)
        }

        function test_breakpoint() {
            verify(Theme.isMobileSize(390, 844), "phone portrait")
            verify(Theme.isMobileSize(844, 390), "phone landscape")
            verify(Theme.isMobileSize(999, 800), "just narrower than the desktop layout")
            verify(Theme.isMobileSize(1600, 500), "wide but short")
            verify(!Theme.isMobileSize(1000, 620), "smallest desktop")
            verify(!Theme.isMobileSize(1440, 900), "desktop")
        }

        function test_tabs() {
            for (var i = 0; i < 4; i++) {
                var t = tab(i)
                verify(t !== null)
                verify(t.height >= 44, "touch-sized tab")
                mouseClick(t)
                compare(root.picked, i)
                compare(bar.current, i)
            }
        }

        function test_beatLight() {
            bar.current = 0
            bar.drumsPlaying = true
            var light = findChild(tab(2), "beatLight")
            verify(light.visible, "drums playing on another tab")
            bar.current = 2
            verify(!light.visible, "not on the drums tab itself")
            bar.drumsPlaying = false
            bar.current = 0
            verify(!light.visible, "drums stopped")
        }
    }
}
