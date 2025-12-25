import 'dart:io';
import 'package:flutter/material.dart';
import 'package:file_picker/file_picker.dart';
import 'package:permission_handler/permission_handler.dart';
import '../models/device.dart';
import '../services/settings_service.dart';
import '../services/sync_service.dart';
import '../services/p2p_service.dart';
import '../services/file_service.dart';

class SettingsScreen extends StatefulWidget {
  final SettingsService settingsService;
  final SyncService syncService;
  final P2PService p2pService;
  final FileService fileService;

  const SettingsScreen({
    super.key,
    required this.settingsService,
    required this.syncService,
    required this.p2pService,
    required this.fileService,
  });

  @override
  State<SettingsScreen> createState() => _SettingsScreenState();
}

class _SettingsScreenState extends State<SettingsScreen> {
  String _deviceName = '';
  int _syncInterval = 10;
  bool _autoSync = true;
  String _syncFolderPath = '';
  List<PairedDevice> _pairedDevices = [];
  bool _isLoading = true;

  final _syncIntervals = [1, 5, 10, 15, 30, 60];

  @override
  void initState() {
    super.initState();
    _loadSettings();
  }

  Future<void> _loadSettings() async {
    final deviceName = await widget.settingsService.getDeviceName();
    final syncInterval = await widget.settingsService.getSyncInterval();
    final autoSync = await widget.settingsService.getAutoSync();
    final syncFolderPath = await widget.settingsService.getSyncFolderPath();
    final pairedDevices = await widget.settingsService.getPairedDevices();

    // Check online status
    for (final device in pairedDevices) {
      device.isOnline = widget.p2pService.isConnected(device.id);
    }

    setState(() {
      _deviceName = deviceName;
      _syncInterval = syncInterval;
      _autoSync = autoSync;
      _syncFolderPath = syncFolderPath ?? widget.fileService.syncPath;
      _pairedDevices = pairedDevices;
      _isLoading = false;
    });
  }

  Future<void> _updateDeviceName() async {
    final controller = TextEditingController(text: _deviceName);

    final newName = await showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Device Name'),
        content: TextField(
          controller: controller,
          decoration: const InputDecoration(
            hintText: 'Enter device name',
          ),
          autofocus: true,
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('Cancel'),
          ),
          TextButton(
            onPressed: () => Navigator.pop(context, controller.text),
            child: const Text('Save'),
          ),
        ],
      ),
    );

    if (newName != null && newName.isNotEmpty) {
      await widget.settingsService.setDeviceName(newName);
      setState(() => _deviceName = newName);
    }
  }

  Future<void> _updateSyncInterval(int minutes) async {
    await widget.settingsService.setSyncInterval(minutes);
    await widget.syncService.updateSyncInterval();
    setState(() => _syncInterval = minutes);
  }

  Future<void> _updateAutoSync(bool enabled) async {
    await widget.settingsService.setAutoSync(enabled);
    if (enabled) {
      await widget.syncService.startAutoSync();
    } else {
      widget.syncService.stopAutoSync();
    }
    setState(() => _autoSync = enabled);
  }

  Future<void> _changeSyncFolder() async {
    final result = await FilePicker.platform.getDirectoryPath(
      dialogTitle: 'Select Sync Folder',
    );

    if (result != null) {
      await widget.settingsService.setSyncFolderPath(result);
      await widget.fileService.changeSyncFolder(result);
      setState(() => _syncFolderPath = result);

      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text('Sync folder changed to: $result'),
            backgroundColor: Colors.green,
          ),
        );
      }
    }
  }

  Future<void> _removePairedDevice(PairedDevice device) async {
    final confirm = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Remove Device'),
        content: Text('Remove "${device.name}" from paired devices?'),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Cancel'),
          ),
          TextButton(
            onPressed: () => Navigator.pop(context, true),
            style: TextButton.styleFrom(foregroundColor: Colors.red),
            child: const Text('Remove'),
          ),
        ],
      ),
    );

    if (confirm == true) {
      widget.p2pService.disconnect(device.id);
      await widget.settingsService.removePairedDevice(device.id);
      _loadSettings();
    }
  }

  @override
  Widget build(BuildContext context) {
    if (_isLoading) {
      return const Scaffold(
        body: Center(child: CircularProgressIndicator()),
      );
    }

    return Scaffold(
      appBar: AppBar(
        title: const Text('Settings'),
      ),
      body: ListView(
        children: [
          // Device Section
          _SectionHeader(title: 'Device'),
          ListTile(
            leading: const Icon(Icons.smartphone),
            title: const Text('Device Name'),
            subtitle: Text(_deviceName),
            trailing: const Icon(Icons.edit),
            onTap: _updateDeviceName,
          ),
          ListTile(
            leading: const Icon(Icons.wifi),
            title: const Text('IP Address'),
            subtitle: Text(widget.p2pService.localIp ?? 'Not available'),
          ),

          const Divider(),

          // Storage Section
          _SectionHeader(title: 'Storage'),
          ListTile(
            leading: const Icon(Icons.folder),
            title: const Text('Sync Folder'),
            subtitle: Text(
              _syncFolderPath,
              overflow: TextOverflow.ellipsis,
              maxLines: 2,
            ),
            trailing: const Icon(Icons.edit),
            onTap: _changeSyncFolder,
          ),
          if (Platform.isAndroid)
            ListTile(
              leading: const Icon(Icons.security),
              title: const Text('Storage Permission'),
              subtitle: const Text('Grant "All files access" for external storage'),
              trailing: ElevatedButton(
                onPressed: () async {
                  await openAppSettings();
                },
                child: const Text('Grant'),
              ),
            ),

          const Divider(),

          // Sync Section
          _SectionHeader(title: 'Sync Settings'),
          SwitchListTile(
            secondary: const Icon(Icons.sync),
            title: const Text('Auto Sync'),
            subtitle: const Text('Automatically sync files at interval'),
            value: _autoSync,
            onChanged: _updateAutoSync,
          ),
          ListTile(
            leading: const Icon(Icons.timer),
            title: const Text('Sync Interval'),
            subtitle: Text('Every $_syncInterval minutes'),
            enabled: _autoSync,
            trailing: DropdownButton<int>(
              value: _syncInterval,
              onChanged: _autoSync
                  ? (value) {
                      if (value != null) _updateSyncInterval(value);
                    }
                  : null,
              items: _syncIntervals.map((minutes) {
                return DropdownMenuItem<int>(
                  value: minutes,
                  child: Text(
                    minutes == 1 ? '1 minute' : '$minutes minutes',
                  ),
                );
              }).toList(),
            ),
          ),
          ListTile(
            leading: const Icon(Icons.info_outline),
            title: const Text('Last Sync'),
            subtitle: Text(
              widget.syncService.lastSync != null
                  ? _formatDateTime(widget.syncService.lastSync!)
                  : 'Never',
            ),
          ),

          const Divider(),

          // Paired Devices Section
          _SectionHeader(title: 'Paired Devices'),
          if (_pairedDevices.isEmpty)
            const ListTile(
              leading: Icon(Icons.devices, color: Colors.grey),
              title: Text(
                'No paired devices',
                style: TextStyle(color: Colors.grey),
              ),
              subtitle: Text('Tap the QR icon to pair a device'),
            )
          else
            ..._pairedDevices.map((device) => ListTile(
                  leading: Icon(
                    Icons.devices,
                    color: device.isOnline ? Colors.green : Colors.grey,
                  ),
                  title: Text(device.name),
                  subtitle: Text(
                    '${device.ipAddress}:${device.port}\n'
                    'Paired: ${_formatDate(device.pairedAt)}',
                  ),
                  isThreeLine: true,
                  trailing: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Container(
                        padding: const EdgeInsets.symmetric(
                          horizontal: 8,
                          vertical: 4,
                        ),
                        decoration: BoxDecoration(
                          color: device.isOnline
                              ? Colors.green.withValues(alpha: 0.1)
                              : Colors.grey.withValues(alpha: 0.1),
                          borderRadius: BorderRadius.circular(12),
                        ),
                        child: Text(
                          device.isOnline ? 'Online' : 'Offline',
                          style: TextStyle(
                            color: device.isOnline ? Colors.green : Colors.grey,
                            fontSize: 12,
                          ),
                        ),
                      ),
                      IconButton(
                        icon: const Icon(Icons.delete_outline),
                        onPressed: () => _removePairedDevice(device),
                      ),
                    ],
                  ),
                )),

          const Divider(),

          // About Section
          _SectionHeader(title: 'About'),
          const ListTile(
            leading: Text('🦉', style: TextStyle(fontSize: 24)),
            title: Text('Owl Transfer'),
            subtitle: Text('Version 1.0.0'),
          ),
          const ListTile(
            leading: Icon(Icons.code),
            title: Text('Direct P2P Sync'),
            subtitle: Text('No cloud storage - files sync directly between devices'),
          ),
        ],
      ),
    );
  }

  String _formatDateTime(DateTime dt) {
    return '${dt.day}/${dt.month}/${dt.year} ${dt.hour.toString().padLeft(2, '0')}:${dt.minute.toString().padLeft(2, '0')}';
  }

  String _formatDate(DateTime dt) {
    return '${dt.day}/${dt.month}/${dt.year}';
  }
}

class _SectionHeader extends StatelessWidget {
  final String title;

  const _SectionHeader({required this.title});

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 16, 16, 8),
      child: Text(
        title,
        style: TextStyle(
          color: Theme.of(context).colorScheme.primary,
          fontWeight: FontWeight.bold,
        ),
      ),
    );
  }
}
