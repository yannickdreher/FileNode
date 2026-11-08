import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Dialogs

ApplicationWindow {
    id: statusWindow
    objectName: "statusWindow"
    title: "FileNode"
    width: 420; height: 620
    visible: true
    flags: App.trayAvailable ? Qt.Tool | Qt.FramelessWindowHint : Qt.Window
    function t(key) { let language = App.language; return App.translate(key) }
    function showPanel() {
        let point = App.popupPosition(width, height); x = point.x; y = point.y
        show(); raise(); requestActivate()
    }
    function showAccountForTest() { settings.showSettings(); pages.currentIndex = 1 }
    onClosing: function(close) { if (App.trayAvailable) { close.accepted = false; hide() } else { App.quit() } }
    header: ToolBar {
        ColumnLayout {
            anchors.fill: parent; anchors.margins: 12
            RowLayout {
                Image { source: "qrc:/filenode.svg"; sourceSize.width: 26; sourceSize.height: 26 }
                Label { text: "FileNode"; font.bold: true; Layout.fillWidth: true }
                ToolButton { text: "☰"; Accessible.name: statusWindow.t("tray.menu"); onClicked: statusMenu.open() }
                ToolButton { text: "×"; Accessible.name: statusWindow.t("tray.hide"); onClicked: App.trayAvailable ? statusWindow.hide() : App.quit() }
            }
            Label { text: App.config.username || statusWindow.t("account.not_connected"); elide: Text.ElideRight; Layout.fillWidth: true }
            Label { text: App.status; wrapMode: Text.Wrap; Layout.fillWidth: true }
        }
    }
    Menu {
        id: statusMenu
        x: statusWindow.width - width - 12; y: 45
        MenuItem { text: statusWindow.t("tray.settings"); onTriggered: settings.showSettings() }
        MenuItem { text: statusWindow.t(App.running ? "action.pause" : "action.resume"); onTriggered: App.running ? App.pause() : App.start() }
        MenuItem { text: statusWindow.t("action.check"); enabled: App.running; onTriggered: App.check() }
        MenuSeparator {}
        MenuItem { text: statusWindow.t("tray.quit"); onTriggered: App.quit() }
    }
    ColumnLayout {
        anchors.fill: parent; anchors.margins: 16; spacing: 12
        Label { visible: !App.running && App.activities.length === 0; text: statusWindow.t("status.welcome"); font.pointSize: 20; wrapMode: Text.Wrap; Layout.fillWidth: true }
        Label { visible: !App.running && App.activities.length === 0; text: statusWindow.t("status.setup_hint"); wrapMode: Text.Wrap; Layout.fillWidth: true }
        Button { visible: !App.running && App.activities.length === 0; text: statusWindow.t("tray.settings"); onClicked: settings.showSettings() }
        Label { visible: App.error.length > 0; text: statusWindow.t("error.summary"); wrapMode: Text.Wrap; Layout.fillWidth: true }
        Label { text: statusWindow.t("activity.recent"); font.bold: true }
        ListView {
            Layout.fillWidth: true; Layout.fillHeight: true; clip: true; spacing: 6
            model: App.activities
            ScrollBar.vertical: ScrollBar {}
            delegate: ItemDelegate {
                width: ListView.view.width
                contentItem: ColumnLayout {
                    Label { text: modelData.name; font.bold: true; Layout.fillWidth: true; elide: Text.ElideMiddle }
                    Label { text: { let language = App.language; return App.formatMessage(modelData.message) } wrapMode: Text.Wrap; Layout.fillWidth: true }
                    Label { text: new Date(modelData.time).toLocaleString(Qt.locale(App.language)); opacity: 0.65; font.pointSize: 9 }
                }
            }
            Label { visible: App.activities.length === 0; text: statusWindow.t("activity.empty"); width: parent.width; wrapMode: Text.Wrap }
        }
    }
    footer: ToolBar {
        RowLayout {
            anchors.fill: parent; anchors.margins: 8
            ToolButton { text: statusWindow.t("action.open"); Layout.fillWidth: true; onClicked: App.openFolder() }
            ToolButton { text: statusWindow.t("action.online"); Layout.fillWidth: true; onClicked: App.openOnline() }
            ToolButton { text: statusWindow.t("action.recovery"); Layout.fillWidth: true; onClicked: App.openRecovery() }
        }
    }

    ApplicationWindow {
        id: settings
        objectName: "settingsWindow"
        title: "FileNode — " + statusWindow.t("tray.settings")
        width: 980; height: 760; minimumWidth: 800; minimumHeight: 640
        visible: startSettings
        onClosing: function(close) { close.accepted = false; hide(); if (!App.trayAvailable) statusWindow.showPanel() }
        function showSettings() { show(); raise(); requestActivate(); if (App.trayAvailable) statusWindow.hide() }
        function save(connectAccount) {
            let browser = authMode.currentIndex === 1
            App.apply({ server: server.text, web_url: web.text, username: username.text,
                propagate_deletes: deletes.checked, interval_seconds: interval.value,
                full_scan_seconds: verification.value,
                auth_mode: browser ? "browser" : "app_password", oauth_issuer:issuer.text,
                oauth_client_id:clientId.text, oauth_scopes:scopes.text, oauth_redirect_port:port.value }, connectAccount, browser ? "" : password.text, browser && connectAccount)
            password.text = ""
        }
        RowLayout {
            anchors.fill: parent; anchors.margins: 20; spacing: 24
            ColumnLayout {
                Layout.minimumWidth: 185; Layout.preferredWidth: 185; Layout.maximumWidth: 185; Layout.fillHeight: true
                Image { source: "qrc:/filenode.svg"; sourceSize.width: 42; sourceSize.height: 42 }
                Label { text: "FileNode"; font.pointSize: 18; font.bold: true }
                Repeater {
                    model: ["settings.sync", "settings.account", "settings.general", "settings.about"]
                    ItemDelegate { text: statusWindow.t(modelData); Layout.fillWidth: true; highlighted: pages.currentIndex === index; onClicked: pages.currentIndex = index }
                }
                Item { Layout.fillHeight: true }
                Label { text: App.version + " · Preview"; opacity: 0.65 }
            }
            ScrollView {
                id: settingsScroll
                Layout.fillWidth: true; Layout.fillHeight: true
                contentWidth: availableWidth; clip: true
                ColumnLayout {
                    width: settingsScroll.availableWidth; spacing: 18
                    Label { text: statusWindow.t(["settings.sync", "settings.account", "settings.general", "settings.about"][pages.currentIndex]); font.pointSize: 24; font.bold: true; wrapMode: Text.Wrap; Layout.fillWidth: true }
                    StackLayout {
                        id: pages; currentIndex: 0; Layout.fillWidth: true
                        ColumnLayout {
                            spacing: 16
                            Label { text: statusWindow.t("settings.sync_hint"); wrapMode: Text.Wrap; Layout.fillWidth: true }
                            Label { text: statusWindow.t("settings.folders"); font.pointSize: 16 }
                            Label { text: statusWindow.t("settings.folders_hint"); wrapMode: Text.Wrap; Layout.fillWidth: true }
                            Label { visible: (App.config.sync_folders || []).length === 0; text: statusWindow.t("settings.no_folders"); wrapMode: Text.Wrap; Layout.fillWidth: true }
                            Repeater {
                                model: App.config.sync_folders || []
                                GroupBox {
                                    required property var modelData
                                    required property int index
                                    Layout.fillWidth: true
                                    ColumnLayout {
                                        anchors.fill: parent
                                        Label { text: modelData.local_folder; font.bold: true; wrapMode: Text.Wrap; Layout.fillWidth: true }
                                        Label { text: statusWindow.t("settings.folder_target") + ": /" + modelData.remote_path; wrapMode: Text.Wrap; Layout.fillWidth: true }
                                        Flow {
                                            Layout.fillWidth: true; spacing: 8
                                            Button { text: statusWindow.t("action.open"); onClicked: App.openPath(modelData.local_folder) }
                                            Button { text: statusWindow.t("action.recovery"); onClicked: App.openPath(modelData.local_folder + "/.filenode/history") }
                                            Button { text: statusWindow.t("settings.remove_folder"); enabled: !App.running; onClicked: { let next = App.config.sync_folders.slice(); next.splice(index, 1); App.apply({sync_folders: next}) } }
                                        }
                                    }
                                }
                            }
                            Button { text: statusWindow.t("settings.add_folder"); enabled: !App.running; onClicked: folderDialog.open() }
                            Label { text: statusWindow.t("settings.stop_hint"); wrapMode: Text.Wrap; Layout.fillWidth: true }
                            Switch { id: deletes; text: statusWindow.t("settings.deletes"); checked: App.config.propagate_deletes || false; enabled: !App.running; Layout.fillWidth: true }
                            GroupBox { title: statusWindow.t("settings.bandwidth"); Layout.fillWidth: true
                                ColumnLayout { anchors.fill: parent
                                    RowLayout {
                                        Switch { id: downloadEnabled; text: statusWindow.t("settings.download_limit"); checked: (App.config.download_limit_kib || 0) > 0; Layout.fillWidth: true; onClicked: App.apply({download_limit_kib: checked ? downloadRate.value : 0}) }
                                        SpinBox { id: downloadRate; from: 1; to: 1000000; value: App.config.download_limit_kib || 1024; enabled: downloadEnabled.checked; editable: true; onValueModified: App.apply({download_limit_kib: value}) }
                                        Label { text: "KiB/s" }
                                    }
                                    RowLayout {
                                        Switch { id: uploadEnabled; text: statusWindow.t("settings.upload_limit"); checked: (App.config.upload_limit_kib || 0) > 0; Layout.fillWidth: true; onClicked: App.apply({upload_limit_kib: checked ? uploadRate.value : 0}) }
                                        SpinBox { id: uploadRate; from: 1; to: 1000000; value: App.config.upload_limit_kib || 1024; enabled: uploadEnabled.checked; editable: true; onValueModified: App.apply({upload_limit_kib: value}) }
                                        Label { text: "KiB/s" }
                                    }
                                }
                            }
                            Switch { text: statusWindow.t("settings.pause_power"); checked: App.config.pause_on_power_saver !== false; Layout.fillWidth: true; onClicked: App.apply({pause_on_power_saver: checked}) }
                            Label { visible: !App.powerAvailable; text: statusWindow.t("settings.power_unavailable"); wrapMode: Text.Wrap; Layout.fillWidth: true }
                            GroupBox { title: statusWindow.t("settings.advanced"); Layout.fillWidth: true
                                GridLayout { anchors.fill: parent; columns: 3
                                    Label { text: statusWindow.t("settings.interval") }
                                    SpinBox { id: interval; from: 5; to: 86400; value: App.config.interval_seconds || 30; enabled: !App.running; editable: true }
                                    Label { text: statusWindow.t("settings.seconds") }
                                    Label { text: statusWindow.t("settings.full_scan") }
                                    SpinBox { id: verification; from: 60; to: 86400; value: App.config.full_scan_seconds || 900; enabled: !App.running; editable: true }
                                    Label { text: statusWindow.t("settings.seconds") }
                                }
                            }
                        }
                        ColumnLayout {
                            spacing: 12
                            Label { text: statusWindow.t("settings.server") }
                            TextField { id: server; text: App.config.server || ""; enabled: !App.running; Layout.fillWidth: true }
                            Label { text: statusWindow.t("settings.web") }
                            TextField { id: web; text: App.config.web_url || ""; enabled: !App.running; Layout.fillWidth: true }
                            Label { text: statusWindow.t("settings.username") }
                            TextField { id: username; text: App.config.username || ""; enabled: !App.running; Layout.fillWidth: true }
                            Label { text: statusWindow.t("settings.authentication") }
                            ComboBox { id: authMode; objectName: "authModeSelector"; model: [statusWindow.t("settings.auth_password"),statusWindow.t("settings.auth_browser")]; currentIndex: App.config.auth_mode === "browser" ? 1 : 0; enabled: !App.running; Layout.fillWidth: true }
                            Label { text: statusWindow.t("settings.password"); visible: authMode.currentIndex === 0 }
                            TextField { id: password; visible: authMode.currentIndex === 0; echoMode: TextInput.Password; placeholderText: statusWindow.t("settings.password_hint"); enabled: !App.running; Layout.fillWidth: true }
                            Label { text: statusWindow.t("settings.credentials"); visible: authMode.currentIndex === 0; wrapMode: Text.Wrap; Layout.fillWidth: true }
                            Label { text: statusWindow.t("settings.browser_hint"); visible: authMode.currentIndex === 1; wrapMode: Text.Wrap; Layout.fillWidth: true }
                            GroupBox { title: statusWindow.t("settings.oauth_advanced"); visible: authMode.currentIndex === 1; Layout.fillWidth: true
                                ColumnLayout { anchors.fill: parent
                                    Label { text: statusWindow.t("settings.oauth_issuer") }
                                    TextField { id: issuer; text: App.config.oauth_issuer || ""; enabled: !App.running; Layout.fillWidth: true }
                                    Label { text: statusWindow.t("settings.oauth_client") }
                                    TextField { id: clientId; text: App.config.oauth_client_id || ""; enabled: !App.running; Layout.fillWidth: true }
                                    Label { text: statusWindow.t("settings.oauth_scopes") }
                                    TextField { id: scopes; text: App.config.oauth_scopes || ""; enabled: !App.running; Layout.fillWidth: true }
                                    Label { text: statusWindow.t("settings.oauth_port") }
                                    SpinBox { id: port; from: 0; to: 65535; value: App.config.oauth_redirect_port || 0; enabled: !App.running; editable: true }
                                }
                            }
                        }
                        ColumnLayout {
                            spacing: 20
                            ComboBox { id: languageChoice; textRole: "label"; valueRole: "code"; model: [{code:"auto", label:statusWindow.t("language.auto")},{code:"en",label:"English"},{code:"de",label:"Deutsch"}]; currentIndex: App.config.language === "de" ? 2 : App.config.language === "en" ? 1 : 0; onActivated: App.apply({language:currentValue}) }
                            Switch { text: statusWindow.t("settings.autostart"); checked: App.config.autostart || false; onClicked: App.apply({autostart:checked}); Layout.fillWidth: true }
                            Label { text: statusWindow.t("settings.background_hint"); wrapMode: Text.Wrap; Layout.fillWidth: true }
                        }
                        ColumnLayout {
                            spacing: 18
                            Label { text: statusWindow.t("about.description"); wrapMode: Text.Wrap; Layout.fillWidth: true }
                            Label { text: statusWindow.t("about.limits"); wrapMode: Text.Wrap; Layout.fillWidth: true }
                            Label { text: "Qt Quick Controls · " + App.style }
                            Button { text: statusWindow.t("about.project"); onClicked: Qt.openUrlExternally("https://github.com/yannickdreher/FileNode") }
                        }
                    }
                    RowLayout {
                        Button { text: statusWindow.t("settings.save"); visible: pages.currentIndex !== 1 && pages.currentIndex !== 3; enabled: App.ready; onClicked: settings.save(false) }
                        Button { objectName: "accountSignInButton"; text: statusWindow.t(App.running ? "action.pause" : "action.signin"); visible: pages.currentIndex === 1; enabled: App.ready; highlighted: true; onClicked: App.running ? App.pause() : settings.save(true) }
                    }
                    Label { visible: App.running; text: statusWindow.t("settings.pause_hint"); wrapMode: Text.Wrap; Layout.fillWidth: true }
                    Label { visible: App.error.length > 0; text: statusWindow.t("error.details") + "\n" + App.error; wrapMode: Text.Wrap; Layout.fillWidth: true; font.family: "monospace" }
                }
            }
        }
        FolderDialog { id: folderDialog; onAccepted: { let next = (App.config.sync_folders || []).slice(); next.push({local_folder:App.localPath(selectedFolder),remote_path:""}); App.apply({sync_folders:next}) } }
    }
    Connections {
        target: App
        function onShowStatus() { statusWindow.showPanel() }
        function onShowSettings() { settings.showSettings() }
        function onShowFolders() { settings.showSettings(); pages.currentIndex = 0 }
        function onHideStatus() { if (App.trayAvailable) statusWindow.hide() }
        function onConfigChanged() {
            // Translated model resets can otherwise overwrite the saved selection with index zero.
            Qt.callLater(function() {
                authMode.currentIndex = App.config.auth_mode === "browser" ? 1 : 0
                languageChoice.currentIndex = App.config.language === "de" ? 2 : App.config.language === "en" ? 1 : 0
            })
        }
    }
    Component.onCompleted: { if (startSettings) settings.showSettings() }
}
