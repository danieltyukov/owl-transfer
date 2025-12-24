import 'dart:convert';
import 'package:shared_preferences/shared_preferences.dart';
import '../models/device.dart';

class SettingsService {
  static const String _keyDeviceId = 'device_id';
  static const String _keyDeviceName = 'device_name';
  static const String _keySyncInterval = 'sync_interval';
  static const String _keyPairedDevices = 'paired_devices';
  static const String _keyAutoSync = 'auto_sync';

  late SharedPreferences _prefs;

  Future<void> initialize() async {
    _prefs = await SharedPreferences.getInstance();
  }

  // Device ID
  Future<String?> getDeviceId() async {
    return _prefs.getString(_keyDeviceId);
  }

  Future<void> setDeviceId(String id) async {
    await _prefs.setString(_keyDeviceId, id);
  }

  // Device Name
  Future<String> getDeviceName() async {
    return _prefs.getString(_keyDeviceName) ?? 'My Device';
  }

  Future<void> setDeviceName(String name) async {
    await _prefs.setString(_keyDeviceName, name);
  }

  // Sync Interval (in minutes)
  Future<int> getSyncInterval() async {
    return _prefs.getInt(_keySyncInterval) ?? 10;
  }

  Future<void> setSyncInterval(int minutes) async {
    await _prefs.setInt(_keySyncInterval, minutes);
  }

  // Auto Sync
  Future<bool> getAutoSync() async {
    return _prefs.getBool(_keyAutoSync) ?? true;
  }

  Future<void> setAutoSync(bool enabled) async {
    await _prefs.setBool(_keyAutoSync, enabled);
  }

  // Paired Devices
  Future<List<PairedDevice>> getPairedDevices() async {
    final json = _prefs.getString(_keyPairedDevices);
    if (json == null) return [];

    final List<dynamic> list = jsonDecode(json);
    return list.map((item) => PairedDevice.fromJson(item)).toList();
  }

  Future<void> addPairedDevice(PairedDevice device) async {
    final devices = await getPairedDevices();

    // Remove if already exists
    devices.removeWhere((d) => d.id == device.id);
    devices.add(device);

    await _savePairedDevices(devices);
  }

  Future<void> removePairedDevice(String deviceId) async {
    final devices = await getPairedDevices();
    devices.removeWhere((d) => d.id == deviceId);
    await _savePairedDevices(devices);
  }

  Future<void> _savePairedDevices(List<PairedDevice> devices) async {
    final json = jsonEncode(devices.map((d) => d.toJson()).toList());
    await _prefs.setString(_keyPairedDevices, json);
  }
}
