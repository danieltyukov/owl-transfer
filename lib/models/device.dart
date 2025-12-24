class PairedDevice {
  final String id;
  final String name;
  final String ipAddress;
  final int port;
  final DateTime pairedAt;
  bool isOnline;

  PairedDevice({
    required this.id,
    required this.name,
    required this.ipAddress,
    required this.port,
    required this.pairedAt,
    this.isOnline = false,
  });

  Map<String, dynamic> toJson() => {
        'id': id,
        'name': name,
        'ipAddress': ipAddress,
        'port': port,
        'pairedAt': pairedAt.toIso8601String(),
      };

  factory PairedDevice.fromJson(Map<String, dynamic> json) => PairedDevice(
        id: json['id'],
        name: json['name'],
        ipAddress: json['ipAddress'],
        port: json['port'],
        pairedAt: DateTime.parse(json['pairedAt']),
      );
}

class PairingInfo {
  final String deviceId;
  final String deviceName;
  final String ipAddress;
  final int port;
  final String pairingCode;

  PairingInfo({
    required this.deviceId,
    required this.deviceName,
    required this.ipAddress,
    required this.port,
    required this.pairingCode,
  });

  Map<String, dynamic> toJson() => {
        'deviceId': deviceId,
        'deviceName': deviceName,
        'ipAddress': ipAddress,
        'port': port,
        'pairingCode': pairingCode,
      };

  factory PairingInfo.fromJson(Map<String, dynamic> json) => PairingInfo(
        deviceId: json['deviceId'],
        deviceName: json['deviceName'],
        ipAddress: json['ipAddress'],
        port: json['port'],
        pairingCode: json['pairingCode'],
      );

  String toQrData() => '${deviceId}|${deviceName}|${ipAddress}|${port}|${pairingCode}';

  factory PairingInfo.fromQrData(String data) {
    final parts = data.split('|');
    return PairingInfo(
      deviceId: parts[0],
      deviceName: parts[1],
      ipAddress: parts[2],
      port: int.parse(parts[3]),
      pairingCode: parts[4],
    );
  }
}
