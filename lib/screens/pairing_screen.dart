import 'dart:io';
import 'package:flutter/material.dart';
import 'package:qr_flutter/qr_flutter.dart';
import 'package:mobile_scanner/mobile_scanner.dart';
import '../models/device.dart';
import '../services/p2p_service.dart';
import '../services/settings_service.dart';

class PairingScreen extends StatefulWidget {
  final P2PService p2pService;
  final SettingsService settingsService;

  const PairingScreen({
    super.key,
    required this.p2pService,
    required this.settingsService,
  });

  @override
  State<PairingScreen> createState() => _PairingScreenState();
}

class _PairingScreenState extends State<PairingScreen>
    with SingleTickerProviderStateMixin {
  late TabController _tabController;
  PairingInfo? _pairingInfo;
  bool _isConnecting = false;
  String? _connectionError;
  bool _isPaired = false;

  @override
  void initState() {
    super.initState();
    _tabController = TabController(length: 2, vsync: this);
    _generatePairingInfo();
    _setupListeners();
  }

  void _generatePairingInfo() {
    setState(() {
      _pairingInfo = widget.p2pService.generatePairingInfo();
    });
  }

  void _setupListeners() {
    widget.p2pService.connectionChanges.listen((event) async {
      final (deviceId, connected) = event;

      if (connected && mounted) {
        // Save paired device
        final devices = await widget.settingsService.getPairedDevices();
        final existingDevice = devices.where((d) => d.id == deviceId).firstOrNull;

        if (existingDevice == null) {
          // Get device info from the pairing info we used to connect
          await widget.settingsService.addPairedDevice(PairedDevice(
            id: deviceId,
            name: 'Paired Device',
            ipAddress: widget.p2pService.localIp ?? '',
            port: P2PService.defaultPort,
            pairedAt: DateTime.now(),
          ));
        }

        setState(() {
          _isPaired = true;
          _isConnecting = false;
        });

        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(
            content: Text('Device paired successfully!'),
            backgroundColor: Colors.green,
          ),
        );

        await Future.delayed(const Duration(seconds: 1));
        if (mounted) {
          Navigator.pop(context);
        }
      }
    });
  }

  Future<void> _onQRScanned(String data) async {
    if (_isConnecting) return;

    setState(() {
      _isConnecting = true;
      _connectionError = null;
    });

    try {
      final pairingInfo = PairingInfo.fromQrData(data);

      final success = await widget.p2pService.connectToPeer(pairingInfo);

      if (!success) {
        setState(() {
          _connectionError = 'Failed to connect to device';
          _isConnecting = false;
        });
      } else {
        // Save paired device
        await widget.settingsService.addPairedDevice(PairedDevice(
          id: pairingInfo.deviceId,
          name: pairingInfo.deviceName,
          ipAddress: pairingInfo.ipAddress,
          port: pairingInfo.port,
          pairedAt: DateTime.now(),
        ));
      }
    } catch (e) {
      setState(() {
        _connectionError = 'Invalid QR code';
        _isConnecting = false;
      });
    }
  }

  @override
  void dispose() {
    _tabController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final isDesktop = Platform.isLinux || Platform.isMacOS || Platform.isWindows;

    return Scaffold(
      appBar: AppBar(
        title: const Text('Pair Device'),
        bottom: TabBar(
          controller: _tabController,
          tabs: [
            const Tab(
              icon: Icon(Icons.qr_code),
              text: 'Show QR',
            ),
            Tab(
              icon: const Icon(Icons.qr_code_scanner),
              text: isDesktop ? 'Enter Code' : 'Scan QR',
            ),
          ],
        ),
      ),
      body: _isPaired
          ? Center(
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  const Icon(
                    Icons.check_circle,
                    color: Colors.green,
                    size: 80,
                  ),
                  const SizedBox(height: 16),
                  const Text(
                    'Device Paired!',
                    style: TextStyle(fontSize: 24, fontWeight: FontWeight.bold),
                  ),
                ],
              ),
            )
          : TabBarView(
              controller: _tabController,
              children: [
                // Show QR Tab
                _buildShowQRTab(),

                // Scan QR Tab
                isDesktop ? _buildManualEntryTab() : _buildScanQRTab(),
              ],
            ),
    );
  }

  Widget _buildShowQRTab() {
    if (_pairingInfo == null) {
      return const Center(child: CircularProgressIndicator());
    }

    return SingleChildScrollView(
      padding: const EdgeInsets.all(24),
      child: Column(
        children: [
          const Text(
            'Scan this QR code from another device',
            style: TextStyle(fontSize: 16),
            textAlign: TextAlign.center,
          ),
          const SizedBox(height: 24),
          Container(
            padding: const EdgeInsets.all(16),
            decoration: BoxDecoration(
              color: Colors.white,
              borderRadius: BorderRadius.circular(16),
              boxShadow: [
                BoxShadow(
                  color: Colors.black.withValues(alpha: 0.1),
                  blurRadius: 10,
                  offset: const Offset(0, 4),
                ),
              ],
            ),
            child: QrImageView(
              data: _pairingInfo!.toQrData(),
              version: QrVersions.auto,
              size: 250,
            ),
          ),
          const SizedBox(height: 24),
          Card(
            child: Padding(
              padding: const EdgeInsets.all(16),
              child: Column(
                children: [
                  _InfoRow(label: 'Device', value: _pairingInfo!.deviceName),
                  _InfoRow(label: 'IP Address', value: _pairingInfo!.ipAddress),
                  _InfoRow(label: 'Port', value: _pairingInfo!.port.toString()),
                  _InfoRow(label: 'Code', value: _pairingInfo!.pairingCode),
                ],
              ),
            ),
          ),
          const SizedBox(height: 16),
          OutlinedButton.icon(
            onPressed: _generatePairingInfo,
            icon: const Icon(Icons.refresh),
            label: const Text('Generate New Code'),
          ),
        ],
      ),
    );
  }

  Widget _buildScanQRTab() {
    return Column(
      children: [
        Expanded(
          child: MobileScanner(
            onDetect: (capture) {
              final List<Barcode> barcodes = capture.barcodes;
              for (final barcode in barcodes) {
                if (barcode.rawValue != null) {
                  _onQRScanned(barcode.rawValue!);
                  break;
                }
              }
            },
          ),
        ),
        if (_isConnecting)
          Container(
            padding: const EdgeInsets.all(16),
            color: Theme.of(context).colorScheme.primaryContainer,
            child: Row(
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                const SizedBox(
                  width: 20,
                  height: 20,
                  child: CircularProgressIndicator(strokeWidth: 2),
                ),
                const SizedBox(width: 16),
                const Text('Connecting...'),
              ],
            ),
          ),
        if (_connectionError != null)
          Container(
            padding: const EdgeInsets.all(16),
            color: Theme.of(context).colorScheme.errorContainer,
            child: Row(
              children: [
                Icon(
                  Icons.error,
                  color: Theme.of(context).colorScheme.error,
                ),
                const SizedBox(width: 16),
                Expanded(
                  child: Text(
                    _connectionError!,
                    style: TextStyle(
                      color: Theme.of(context).colorScheme.onErrorContainer,
                    ),
                  ),
                ),
              ],
            ),
          ),
      ],
    );
  }

  Widget _buildManualEntryTab() {
    final ipController = TextEditingController();
    final portController = TextEditingController(text: '52734');
    final codeController = TextEditingController();

    return SingleChildScrollView(
      padding: const EdgeInsets.all(24),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Text(
            'Enter the connection details from the other device:',
            style: TextStyle(fontSize: 16),
          ),
          const SizedBox(height: 24),
          TextField(
            controller: ipController,
            decoration: const InputDecoration(
              labelText: 'IP Address',
              hintText: '192.168.1.100',
              border: OutlineInputBorder(),
            ),
            keyboardType: TextInputType.number,
          ),
          const SizedBox(height: 16),
          TextField(
            controller: portController,
            decoration: const InputDecoration(
              labelText: 'Port',
              border: OutlineInputBorder(),
            ),
            keyboardType: TextInputType.number,
          ),
          const SizedBox(height: 16),
          TextField(
            controller: codeController,
            decoration: const InputDecoration(
              labelText: 'Pairing Code',
              border: OutlineInputBorder(),
            ),
          ),
          const SizedBox(height: 24),
          SizedBox(
            width: double.infinity,
            child: ElevatedButton.icon(
              onPressed: _isConnecting
                  ? null
                  : () {
                      final data =
                          'manual|Desktop|${ipController.text}|${portController.text}|${codeController.text}';
                      _onQRScanned(data);
                    },
              icon: _isConnecting
                  ? const SizedBox(
                      width: 20,
                      height: 20,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    )
                  : const Icon(Icons.link),
              label: Text(_isConnecting ? 'Connecting...' : 'Connect'),
            ),
          ),
          if (_connectionError != null) ...[
            const SizedBox(height: 16),
            Container(
              padding: const EdgeInsets.all(12),
              decoration: BoxDecoration(
                color: Theme.of(context).colorScheme.errorContainer,
                borderRadius: BorderRadius.circular(8),
              ),
              child: Row(
                children: [
                  Icon(
                    Icons.error,
                    color: Theme.of(context).colorScheme.error,
                  ),
                  const SizedBox(width: 12),
                  Expanded(
                    child: Text(
                      _connectionError!,
                      style: TextStyle(
                        color: Theme.of(context).colorScheme.onErrorContainer,
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ],
        ],
      ),
    );
  }
}

class _InfoRow extends StatelessWidget {
  final String label;
  final String value;

  const _InfoRow({required this.label, required this.value});

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          Text(
            label,
            style: TextStyle(color: Colors.grey[600]),
          ),
          Text(
            value,
            style: const TextStyle(fontWeight: FontWeight.w500),
          ),
        ],
      ),
    );
  }
}
