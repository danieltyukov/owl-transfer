import 'dart:io';
import 'package:flutter/material.dart';
import 'services/file_service.dart';
import 'services/p2p_service.dart';
import 'services/settings_service.dart';
import 'services/sync_service.dart';
import 'screens/home_screen.dart';
import 'models/device.dart';

void main() async {
  WidgetsFlutterBinding.ensureInitialized();

  runApp(const OwlTransferApp());
}

class OwlTransferApp extends StatelessWidget {
  const OwlTransferApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Owl Transfer',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        colorScheme: ColorScheme.fromSeed(
          seedColor: const Color(0xFF8B5A2B), // Owl brown
          brightness: Brightness.light,
        ),
        useMaterial3: true,
        appBarTheme: const AppBarTheme(
          centerTitle: false,
          elevation: 0,
        ),
      ),
      darkTheme: ThemeData(
        colorScheme: ColorScheme.fromSeed(
          seedColor: const Color(0xFF8B5A2B),
          brightness: Brightness.dark,
        ),
        useMaterial3: true,
        appBarTheme: const AppBarTheme(
          centerTitle: false,
          elevation: 0,
        ),
      ),
      themeMode: ThemeMode.system,
      home: const AppInitializer(),
    );
  }
}

class AppInitializer extends StatefulWidget {
  const AppInitializer({super.key});

  @override
  State<AppInitializer> createState() => _AppInitializerState();
}

class _AppInitializerState extends State<AppInitializer> {
  late FileService _fileService;
  late P2PService _p2pService;
  late SettingsService _settingsService;
  late SyncService _syncService;

  bool _isInitialized = false;
  String? _initError;

  @override
  void initState() {
    super.initState();
    _initialize();
  }

  Future<void> _initialize() async {
    try {
      // Initialize settings first
      _settingsService = SettingsService();
      await _settingsService.initialize();

      // Get or create device name
      String deviceName = await _settingsService.getDeviceName();
      if (deviceName == 'My Device') {
        // Set a more descriptive default name
        deviceName = Platform.isAndroid
            ? 'Android Phone'
            : Platform.isLinux
                ? 'Linux Desktop'
                : 'My Device';
        await _settingsService.setDeviceName(deviceName);
      }

      // Get saved sync folder path
      final syncFolderPath = await _settingsService.getSyncFolderPath();

      // Initialize file service with custom path if set
      _fileService = FileService();
      await _fileService.initialize(customPath: syncFolderPath);

      // Initialize P2P service
      _p2pService = P2PService();
      await _p2pService.initialize(deviceName);
      await _p2pService.startServer();

      // Initialize sync service
      _syncService = SyncService(_fileService, _p2pService, _settingsService);

      // Start auto-sync if enabled
      final autoSync = await _settingsService.getAutoSync();
      if (autoSync) {
        await _syncService.startAutoSync();
      }

      // Try to reconnect to paired devices
      final pairedDevices = await _settingsService.getPairedDevices();
      for (final device in pairedDevices) {
        // Try to connect in background
        _p2pService.connectToPeer(
          PairingInfo(
            deviceId: device.id,
            deviceName: device.name,
            ipAddress: device.ipAddress,
            port: device.port,
            pairingCode: '', // Empty code for reconnection attempt
          ),
        ).catchError((_) {
          // Ignore connection failures on startup
        });
      }

      setState(() => _isInitialized = true);
    } catch (e) {
      setState(() => _initError = e.toString());
    }
  }

  @override
  void dispose() {
    if (_isInitialized) {
      _syncService.dispose();
      _p2pService.dispose();
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    if (_initError != null) {
      return Scaffold(
        body: Center(
          child: Padding(
            padding: const EdgeInsets.all(24),
            child: Column(
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                const Icon(
                  Icons.error_outline,
                  size: 64,
                  color: Colors.red,
                ),
                const SizedBox(height: 16),
                const Text(
                  'Failed to initialize',
                  style: TextStyle(
                    fontSize: 20,
                    fontWeight: FontWeight.bold,
                  ),
                ),
                const SizedBox(height: 8),
                Text(
                  _initError!,
                  textAlign: TextAlign.center,
                  style: const TextStyle(color: Colors.grey),
                ),
                const SizedBox(height: 24),
                ElevatedButton(
                  onPressed: () {
                    setState(() => _initError = null);
                    _initialize();
                  },
                  child: const Text('Retry'),
                ),
              ],
            ),
          ),
        ),
      );
    }

    if (!_isInitialized) {
      return const Scaffold(
        body: Center(
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              Text('🦉', style: TextStyle(fontSize: 64)),
              SizedBox(height: 24),
              Text(
                'Owl Transfer',
                style: TextStyle(
                  fontSize: 24,
                  fontWeight: FontWeight.bold,
                ),
              ),
              SizedBox(height: 24),
              CircularProgressIndicator(),
              SizedBox(height: 16),
              Text('Starting up...'),
            ],
          ),
        ),
      );
    }

    return HomeScreen(
      fileService: _fileService,
      syncService: _syncService,
      p2pService: _p2pService,
      settingsService: _settingsService,
    );
  }
}
