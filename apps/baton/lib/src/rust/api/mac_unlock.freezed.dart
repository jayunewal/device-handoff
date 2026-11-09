// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint, type=warning, deprecated_member_use, deprecated_member_use_from_same_package
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'mac_unlock.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// dart format off
T _$identity<T>(T value) => value;
/// @nodoc
mixin _$SshKeyCheck {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is SshKeyCheck);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'SshKeyCheck()';
}


}

/// @nodoc
class $SshKeyCheckCopyWith<$Res>  {
$SshKeyCheckCopyWith(SshKeyCheck _, $Res Function(SshKeyCheck) __);
}


/// Adds pattern-matching-related methods to [SshKeyCheck].
extension SshKeyCheckPatterns on SshKeyCheck {
/// A variant of `map` that fallback to returning `orElse`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( SshKeyCheck_Heard value)?  heard,TResult Function( SshKeyCheck_Unreachable value)?  unreachable,TResult Function( SshKeyCheck_Failed value)?  failed,required TResult orElse(),}){
final _that = this;
switch (_that) {
case SshKeyCheck_Heard() when heard != null:
return heard(_that);case SshKeyCheck_Unreachable() when unreachable != null:
return unreachable(_that);case SshKeyCheck_Failed() when failed != null:
return failed(_that);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// Callbacks receives the raw object, upcasted.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case final Subclass2 value:
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( SshKeyCheck_Heard value)  heard,required TResult Function( SshKeyCheck_Unreachable value)  unreachable,required TResult Function( SshKeyCheck_Failed value)  failed,}){
final _that = this;
switch (_that) {
case SshKeyCheck_Heard():
return heard(_that);case SshKeyCheck_Unreachable():
return unreachable(_that);case SshKeyCheck_Failed():
return failed(_that);}
}
/// A variant of `map` that fallback to returning `null`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( SshKeyCheck_Heard value)?  heard,TResult? Function( SshKeyCheck_Unreachable value)?  unreachable,TResult? Function( SshKeyCheck_Failed value)?  failed,}){
final _that = this;
switch (_that) {
case SshKeyCheck_Heard() when heard != null:
return heard(_that);case SshKeyCheck_Unreachable() when unreachable != null:
return unreachable(_that);case SshKeyCheck_Failed() when failed != null:
return failed(_that);case _:
  return null;

}
}
/// A variant of `when` that fallback to an `orElse` callback.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( String fingerprint)?  heard,TResult Function( String detail)?  unreachable,TResult Function( String detail)?  failed,required TResult orElse(),}) {final _that = this;
switch (_that) {
case SshKeyCheck_Heard() when heard != null:
return heard(_that.fingerprint);case SshKeyCheck_Unreachable() when unreachable != null:
return unreachable(_that.detail);case SshKeyCheck_Failed() when failed != null:
return failed(_that.detail);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// As opposed to `map`, this offers destructuring.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case Subclass2(:final field2):
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( String fingerprint)  heard,required TResult Function( String detail)  unreachable,required TResult Function( String detail)  failed,}) {final _that = this;
switch (_that) {
case SshKeyCheck_Heard():
return heard(_that.fingerprint);case SshKeyCheck_Unreachable():
return unreachable(_that.detail);case SshKeyCheck_Failed():
return failed(_that.detail);}
}
/// A variant of `when` that fallback to returning `null`
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( String fingerprint)?  heard,TResult? Function( String detail)?  unreachable,TResult? Function( String detail)?  failed,}) {final _that = this;
switch (_that) {
case SshKeyCheck_Heard() when heard != null:
return heard(_that.fingerprint);case SshKeyCheck_Unreachable() when unreachable != null:
return unreachable(_that.detail);case SshKeyCheck_Failed() when failed != null:
return failed(_that.detail);case _:
  return null;

}
}

}

/// @nodoc


class SshKeyCheck_Heard extends SshKeyCheck {
  const SshKeyCheck_Heard({required this.fingerprint}): super._();
  

/// The key's fingerprint.
 final  String fingerprint;

/// Create a copy of SshKeyCheck
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$SshKeyCheck_HeardCopyWith<SshKeyCheck_Heard> get copyWith => _$SshKeyCheck_HeardCopyWithImpl<SshKeyCheck_Heard>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is SshKeyCheck_Heard&&(identical(other.fingerprint, fingerprint) || other.fingerprint == fingerprint));
}


@override
int get hashCode {
    return Object.hash(runtimeType,fingerprint);
}

@override
String toString() {
    return 'SshKeyCheck.heard(fingerprint: $fingerprint)';
}


}

/// @nodoc
abstract mixin class $SshKeyCheck_HeardCopyWith<$Res> implements $SshKeyCheckCopyWith<$Res> {
  factory $SshKeyCheck_HeardCopyWith(SshKeyCheck_Heard value, $Res Function(SshKeyCheck_Heard) _then) = _$SshKeyCheck_HeardCopyWithImpl;
@useResult
$Res call({
 String fingerprint
});




}
/// @nodoc
class _$SshKeyCheck_HeardCopyWithImpl<$Res>
    implements $SshKeyCheck_HeardCopyWith<$Res> {
  _$SshKeyCheck_HeardCopyWithImpl(this._self, this._then);

  final SshKeyCheck_Heard _self;
  final $Res Function(SshKeyCheck_Heard) _then;

/// Create a copy of SshKeyCheck
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? fingerprint = null,}) {
  return _then(SshKeyCheck_Heard(
fingerprint: null == fingerprint ? _self.fingerprint : fingerprint // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class SshKeyCheck_Unreachable extends SshKeyCheck {
  const SshKeyCheck_Unreachable({required this.detail}): super._();
  

/// What failed, for a log.
 final  String detail;

/// Create a copy of SshKeyCheck
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$SshKeyCheck_UnreachableCopyWith<SshKeyCheck_Unreachable> get copyWith => _$SshKeyCheck_UnreachableCopyWithImpl<SshKeyCheck_Unreachable>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is SshKeyCheck_Unreachable&&(identical(other.detail, detail) || other.detail == detail));
}


@override
int get hashCode {
    return Object.hash(runtimeType,detail);
}

@override
String toString() {
    return 'SshKeyCheck.unreachable(detail: $detail)';
}


}

/// @nodoc
abstract mixin class $SshKeyCheck_UnreachableCopyWith<$Res> implements $SshKeyCheckCopyWith<$Res> {
  factory $SshKeyCheck_UnreachableCopyWith(SshKeyCheck_Unreachable value, $Res Function(SshKeyCheck_Unreachable) _then) = _$SshKeyCheck_UnreachableCopyWithImpl;
@useResult
$Res call({
 String detail
});




}
/// @nodoc
class _$SshKeyCheck_UnreachableCopyWithImpl<$Res>
    implements $SshKeyCheck_UnreachableCopyWith<$Res> {
  _$SshKeyCheck_UnreachableCopyWithImpl(this._self, this._then);

  final SshKeyCheck_Unreachable _self;
  final $Res Function(SshKeyCheck_Unreachable) _then;

/// Create a copy of SshKeyCheck
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? detail = null,}) {
  return _then(SshKeyCheck_Unreachable(
detail: null == detail ? _self.detail : detail // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class SshKeyCheck_Failed extends SshKeyCheck {
  const SshKeyCheck_Failed({required this.detail}): super._();
  

/// What failed, for a log.
 final  String detail;

/// Create a copy of SshKeyCheck
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$SshKeyCheck_FailedCopyWith<SshKeyCheck_Failed> get copyWith => _$SshKeyCheck_FailedCopyWithImpl<SshKeyCheck_Failed>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is SshKeyCheck_Failed&&(identical(other.detail, detail) || other.detail == detail));
}


@override
int get hashCode {
    return Object.hash(runtimeType,detail);
}

@override
String toString() {
    return 'SshKeyCheck.failed(detail: $detail)';
}


}

/// @nodoc
abstract mixin class $SshKeyCheck_FailedCopyWith<$Res> implements $SshKeyCheckCopyWith<$Res> {
  factory $SshKeyCheck_FailedCopyWith(SshKeyCheck_Failed value, $Res Function(SshKeyCheck_Failed) _then) = _$SshKeyCheck_FailedCopyWithImpl;
@useResult
$Res call({
 String detail
});




}
/// @nodoc
class _$SshKeyCheck_FailedCopyWithImpl<$Res>
    implements $SshKeyCheck_FailedCopyWith<$Res> {
  _$SshKeyCheck_FailedCopyWithImpl(this._self, this._then);

  final SshKeyCheck_Failed _self;
  final $Res Function(SshKeyCheck_Failed) _then;

/// Create a copy of SshKeyCheck
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? detail = null,}) {
  return _then(SshKeyCheck_Failed(
detail: null == detail ? _self.detail : detail // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc
mixin _$SshUnlock {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is SshUnlock);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'SshUnlock()';
}


}

/// @nodoc
class $SshUnlockCopyWith<$Res>  {
$SshUnlockCopyWith(SshUnlock _, $Res Function(SshUnlock) __);
}


/// Adds pattern-matching-related methods to [SshUnlock].
extension SshUnlockPatterns on SshUnlock {
/// A variant of `map` that fallback to returning `orElse`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( SshUnlock_Accepted value)?  accepted,TResult Function( SshUnlock_Rejected value)?  rejected,TResult Function( SshUnlock_UnknownKey value)?  unknownKey,TResult Function( SshUnlock_NoPassword value)?  noPassword,TResult Function( SshUnlock_Unreachable value)?  unreachable,TResult Function( SshUnlock_Failed value)?  failed,required TResult orElse(),}){
final _that = this;
switch (_that) {
case SshUnlock_Accepted() when accepted != null:
return accepted(_that);case SshUnlock_Rejected() when rejected != null:
return rejected(_that);case SshUnlock_UnknownKey() when unknownKey != null:
return unknownKey(_that);case SshUnlock_NoPassword() when noPassword != null:
return noPassword(_that);case SshUnlock_Unreachable() when unreachable != null:
return unreachable(_that);case SshUnlock_Failed() when failed != null:
return failed(_that);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// Callbacks receives the raw object, upcasted.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case final Subclass2 value:
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( SshUnlock_Accepted value)  accepted,required TResult Function( SshUnlock_Rejected value)  rejected,required TResult Function( SshUnlock_UnknownKey value)  unknownKey,required TResult Function( SshUnlock_NoPassword value)  noPassword,required TResult Function( SshUnlock_Unreachable value)  unreachable,required TResult Function( SshUnlock_Failed value)  failed,}){
final _that = this;
switch (_that) {
case SshUnlock_Accepted():
return accepted(_that);case SshUnlock_Rejected():
return rejected(_that);case SshUnlock_UnknownKey():
return unknownKey(_that);case SshUnlock_NoPassword():
return noPassword(_that);case SshUnlock_Unreachable():
return unreachable(_that);case SshUnlock_Failed():
return failed(_that);}
}
/// A variant of `map` that fallback to returning `null`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( SshUnlock_Accepted value)?  accepted,TResult? Function( SshUnlock_Rejected value)?  rejected,TResult? Function( SshUnlock_UnknownKey value)?  unknownKey,TResult? Function( SshUnlock_NoPassword value)?  noPassword,TResult? Function( SshUnlock_Unreachable value)?  unreachable,TResult? Function( SshUnlock_Failed value)?  failed,}){
final _that = this;
switch (_that) {
case SshUnlock_Accepted() when accepted != null:
return accepted(_that);case SshUnlock_Rejected() when rejected != null:
return rejected(_that);case SshUnlock_UnknownKey() when unknownKey != null:
return unknownKey(_that);case SshUnlock_NoPassword() when noPassword != null:
return noPassword(_that);case SshUnlock_Unreachable() when unreachable != null:
return unreachable(_that);case SshUnlock_Failed() when failed != null:
return failed(_that);case _:
  return null;

}
}
/// A variant of `when` that fallback to an `orElse` callback.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function()?  accepted,TResult Function()?  rejected,TResult Function( String fingerprint)?  unknownKey,TResult Function()?  noPassword,TResult Function( String detail)?  unreachable,TResult Function( String detail)?  failed,required TResult orElse(),}) {final _that = this;
switch (_that) {
case SshUnlock_Accepted() when accepted != null:
return accepted();case SshUnlock_Rejected() when rejected != null:
return rejected();case SshUnlock_UnknownKey() when unknownKey != null:
return unknownKey(_that.fingerprint);case SshUnlock_NoPassword() when noPassword != null:
return noPassword();case SshUnlock_Unreachable() when unreachable != null:
return unreachable(_that.detail);case SshUnlock_Failed() when failed != null:
return failed(_that.detail);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// As opposed to `map`, this offers destructuring.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case Subclass2(:final field2):
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function()  accepted,required TResult Function()  rejected,required TResult Function( String fingerprint)  unknownKey,required TResult Function()  noPassword,required TResult Function( String detail)  unreachable,required TResult Function( String detail)  failed,}) {final _that = this;
switch (_that) {
case SshUnlock_Accepted():
return accepted();case SshUnlock_Rejected():
return rejected();case SshUnlock_UnknownKey():
return unknownKey(_that.fingerprint);case SshUnlock_NoPassword():
return noPassword();case SshUnlock_Unreachable():
return unreachable(_that.detail);case SshUnlock_Failed():
return failed(_that.detail);}
}
/// A variant of `when` that fallback to returning `null`
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function()?  accepted,TResult? Function()?  rejected,TResult? Function( String fingerprint)?  unknownKey,TResult? Function()?  noPassword,TResult? Function( String detail)?  unreachable,TResult? Function( String detail)?  failed,}) {final _that = this;
switch (_that) {
case SshUnlock_Accepted() when accepted != null:
return accepted();case SshUnlock_Rejected() when rejected != null:
return rejected();case SshUnlock_UnknownKey() when unknownKey != null:
return unknownKey(_that.fingerprint);case SshUnlock_NoPassword() when noPassword != null:
return noPassword();case SshUnlock_Unreachable() when unreachable != null:
return unreachable(_that.detail);case SshUnlock_Failed() when failed != null:
return failed(_that.detail);case _:
  return null;

}
}

}

/// @nodoc


class SshUnlock_Accepted extends SshUnlock {
  const SshUnlock_Accepted(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is SshUnlock_Accepted);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'SshUnlock.accepted()';
}


}




/// @nodoc


class SshUnlock_Rejected extends SshUnlock {
  const SshUnlock_Rejected(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is SshUnlock_Rejected);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'SshUnlock.rejected()';
}


}




/// @nodoc


class SshUnlock_UnknownKey extends SshUnlock {
  const SshUnlock_UnknownKey({required this.fingerprint}): super._();
  

/// The fingerprint of the key it answered with.
 final  String fingerprint;

/// Create a copy of SshUnlock
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$SshUnlock_UnknownKeyCopyWith<SshUnlock_UnknownKey> get copyWith => _$SshUnlock_UnknownKeyCopyWithImpl<SshUnlock_UnknownKey>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is SshUnlock_UnknownKey&&(identical(other.fingerprint, fingerprint) || other.fingerprint == fingerprint));
}


@override
int get hashCode {
    return Object.hash(runtimeType,fingerprint);
}

@override
String toString() {
    return 'SshUnlock.unknownKey(fingerprint: $fingerprint)';
}


}

/// @nodoc
abstract mixin class $SshUnlock_UnknownKeyCopyWith<$Res> implements $SshUnlockCopyWith<$Res> {
  factory $SshUnlock_UnknownKeyCopyWith(SshUnlock_UnknownKey value, $Res Function(SshUnlock_UnknownKey) _then) = _$SshUnlock_UnknownKeyCopyWithImpl;
@useResult
$Res call({
 String fingerprint
});




}
/// @nodoc
class _$SshUnlock_UnknownKeyCopyWithImpl<$Res>
    implements $SshUnlock_UnknownKeyCopyWith<$Res> {
  _$SshUnlock_UnknownKeyCopyWithImpl(this._self, this._then);

  final SshUnlock_UnknownKey _self;
  final $Res Function(SshUnlock_UnknownKey) _then;

/// Create a copy of SshUnlock
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? fingerprint = null,}) {
  return _then(SshUnlock_UnknownKey(
fingerprint: null == fingerprint ? _self.fingerprint : fingerprint // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class SshUnlock_NoPassword extends SshUnlock {
  const SshUnlock_NoPassword(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is SshUnlock_NoPassword);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'SshUnlock.noPassword()';
}


}




/// @nodoc


class SshUnlock_Unreachable extends SshUnlock {
  const SshUnlock_Unreachable({required this.detail}): super._();
  

/// What failed, for a log.
 final  String detail;

/// Create a copy of SshUnlock
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$SshUnlock_UnreachableCopyWith<SshUnlock_Unreachable> get copyWith => _$SshUnlock_UnreachableCopyWithImpl<SshUnlock_Unreachable>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is SshUnlock_Unreachable&&(identical(other.detail, detail) || other.detail == detail));
}


@override
int get hashCode {
    return Object.hash(runtimeType,detail);
}

@override
String toString() {
    return 'SshUnlock.unreachable(detail: $detail)';
}


}

/// @nodoc
abstract mixin class $SshUnlock_UnreachableCopyWith<$Res> implements $SshUnlockCopyWith<$Res> {
  factory $SshUnlock_UnreachableCopyWith(SshUnlock_Unreachable value, $Res Function(SshUnlock_Unreachable) _then) = _$SshUnlock_UnreachableCopyWithImpl;
@useResult
$Res call({
 String detail
});




}
/// @nodoc
class _$SshUnlock_UnreachableCopyWithImpl<$Res>
    implements $SshUnlock_UnreachableCopyWith<$Res> {
  _$SshUnlock_UnreachableCopyWithImpl(this._self, this._then);

  final SshUnlock_Unreachable _self;
  final $Res Function(SshUnlock_Unreachable) _then;

/// Create a copy of SshUnlock
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? detail = null,}) {
  return _then(SshUnlock_Unreachable(
detail: null == detail ? _self.detail : detail // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class SshUnlock_Failed extends SshUnlock {
  const SshUnlock_Failed({required this.detail}): super._();
  

/// What failed, for a log.
 final  String detail;

/// Create a copy of SshUnlock
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$SshUnlock_FailedCopyWith<SshUnlock_Failed> get copyWith => _$SshUnlock_FailedCopyWithImpl<SshUnlock_Failed>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is SshUnlock_Failed&&(identical(other.detail, detail) || other.detail == detail));
}


@override
int get hashCode {
    return Object.hash(runtimeType,detail);
}

@override
String toString() {
    return 'SshUnlock.failed(detail: $detail)';
}


}

/// @nodoc
abstract mixin class $SshUnlock_FailedCopyWith<$Res> implements $SshUnlockCopyWith<$Res> {
  factory $SshUnlock_FailedCopyWith(SshUnlock_Failed value, $Res Function(SshUnlock_Failed) _then) = _$SshUnlock_FailedCopyWithImpl;
@useResult
$Res call({
 String detail
});




}
/// @nodoc
class _$SshUnlock_FailedCopyWithImpl<$Res>
    implements $SshUnlock_FailedCopyWith<$Res> {
  _$SshUnlock_FailedCopyWithImpl(this._self, this._then);

  final SshUnlock_Failed _self;
  final $Res Function(SshUnlock_Failed) _then;

/// Create a copy of SshUnlock
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? detail = null,}) {
  return _then(SshUnlock_Failed(
detail: null == detail ? _self.detail : detail // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

// dart format on
