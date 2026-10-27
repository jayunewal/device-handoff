// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint, type=warning, deprecated_member_use, deprecated_member_use_from_same_package
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'model.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// dart format off
T _$identity<T>(T value) => value;
/// @nodoc
mixin _$Event {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Event);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'Event()';
}


}

/// @nodoc
class $EventCopyWith<$Res>  {
$EventCopyWith(Event _, $Res Function(Event) __);
}


/// Adds pattern-matching-related methods to [Event].
extension EventPatterns on Event {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( Event_Received value)?  received,TResult Function( Event_DevicesChanged value)?  devicesChanged,TResult Function( Event_PairingRequested value)?  pairingRequested,TResult Function( Event_PairingRequestEnded value)?  pairingRequestEnded,TResult Function( Event_OfferEnded value)?  offerEnded,TResult Function( Event_TransferRequested value)?  transferRequested,TResult Function( Event_TransferRequestEnded value)?  transferRequestEnded,TResult Function( Event_TransferEnded value)?  transferEnded,TResult Function( Event_WindowRequested value)?  windowRequested,TResult Function( Event_WindowRequestEnded value)?  windowRequestEnded,TResult Function( Event_WindowStarted value)?  windowStarted,TResult Function( Event_WindowPicture value)?  windowPicture,TResult Function( Event_WindowNotice value)?  windowNotice,TResult Function( Event_WindowSound value)?  windowSound,TResult Function( Event_WindowLocked value)?  windowLocked,TResult Function( Event_WindowUnlock value)?  windowUnlock,TResult Function( Event_UnlockAttempted value)?  unlockAttempted,TResult Function( Event_WindowEnded value)?  windowEnded,TResult Function( Event_Resync value)?  resync,required TResult orElse(),}){
final _that = this;
switch (_that) {
case Event_Received() when received != null:
return received(_that);case Event_DevicesChanged() when devicesChanged != null:
return devicesChanged(_that);case Event_PairingRequested() when pairingRequested != null:
return pairingRequested(_that);case Event_PairingRequestEnded() when pairingRequestEnded != null:
return pairingRequestEnded(_that);case Event_OfferEnded() when offerEnded != null:
return offerEnded(_that);case Event_TransferRequested() when transferRequested != null:
return transferRequested(_that);case Event_TransferRequestEnded() when transferRequestEnded != null:
return transferRequestEnded(_that);case Event_TransferEnded() when transferEnded != null:
return transferEnded(_that);case Event_WindowRequested() when windowRequested != null:
return windowRequested(_that);case Event_WindowRequestEnded() when windowRequestEnded != null:
return windowRequestEnded(_that);case Event_WindowStarted() when windowStarted != null:
return windowStarted(_that);case Event_WindowPicture() when windowPicture != null:
return windowPicture(_that);case Event_WindowNotice() when windowNotice != null:
return windowNotice(_that);case Event_WindowSound() when windowSound != null:
return windowSound(_that);case Event_WindowLocked() when windowLocked != null:
return windowLocked(_that);case Event_WindowUnlock() when windowUnlock != null:
return windowUnlock(_that);case Event_UnlockAttempted() when unlockAttempted != null:
return unlockAttempted(_that);case Event_WindowEnded() when windowEnded != null:
return windowEnded(_that);case Event_Resync() when resync != null:
return resync(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( Event_Received value)  received,required TResult Function( Event_DevicesChanged value)  devicesChanged,required TResult Function( Event_PairingRequested value)  pairingRequested,required TResult Function( Event_PairingRequestEnded value)  pairingRequestEnded,required TResult Function( Event_OfferEnded value)  offerEnded,required TResult Function( Event_TransferRequested value)  transferRequested,required TResult Function( Event_TransferRequestEnded value)  transferRequestEnded,required TResult Function( Event_TransferEnded value)  transferEnded,required TResult Function( Event_WindowRequested value)  windowRequested,required TResult Function( Event_WindowRequestEnded value)  windowRequestEnded,required TResult Function( Event_WindowStarted value)  windowStarted,required TResult Function( Event_WindowPicture value)  windowPicture,required TResult Function( Event_WindowNotice value)  windowNotice,required TResult Function( Event_WindowSound value)  windowSound,required TResult Function( Event_WindowLocked value)  windowLocked,required TResult Function( Event_WindowUnlock value)  windowUnlock,required TResult Function( Event_UnlockAttempted value)  unlockAttempted,required TResult Function( Event_WindowEnded value)  windowEnded,required TResult Function( Event_Resync value)  resync,}){
final _that = this;
switch (_that) {
case Event_Received():
return received(_that);case Event_DevicesChanged():
return devicesChanged(_that);case Event_PairingRequested():
return pairingRequested(_that);case Event_PairingRequestEnded():
return pairingRequestEnded(_that);case Event_OfferEnded():
return offerEnded(_that);case Event_TransferRequested():
return transferRequested(_that);case Event_TransferRequestEnded():
return transferRequestEnded(_that);case Event_TransferEnded():
return transferEnded(_that);case Event_WindowRequested():
return windowRequested(_that);case Event_WindowRequestEnded():
return windowRequestEnded(_that);case Event_WindowStarted():
return windowStarted(_that);case Event_WindowPicture():
return windowPicture(_that);case Event_WindowNotice():
return windowNotice(_that);case Event_WindowSound():
return windowSound(_that);case Event_WindowLocked():
return windowLocked(_that);case Event_WindowUnlock():
return windowUnlock(_that);case Event_UnlockAttempted():
return unlockAttempted(_that);case Event_WindowEnded():
return windowEnded(_that);case Event_Resync():
return resync(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( Event_Received value)?  received,TResult? Function( Event_DevicesChanged value)?  devicesChanged,TResult? Function( Event_PairingRequested value)?  pairingRequested,TResult? Function( Event_PairingRequestEnded value)?  pairingRequestEnded,TResult? Function( Event_OfferEnded value)?  offerEnded,TResult? Function( Event_TransferRequested value)?  transferRequested,TResult? Function( Event_TransferRequestEnded value)?  transferRequestEnded,TResult? Function( Event_TransferEnded value)?  transferEnded,TResult? Function( Event_WindowRequested value)?  windowRequested,TResult? Function( Event_WindowRequestEnded value)?  windowRequestEnded,TResult? Function( Event_WindowStarted value)?  windowStarted,TResult? Function( Event_WindowPicture value)?  windowPicture,TResult? Function( Event_WindowNotice value)?  windowNotice,TResult? Function( Event_WindowSound value)?  windowSound,TResult? Function( Event_WindowLocked value)?  windowLocked,TResult? Function( Event_WindowUnlock value)?  windowUnlock,TResult? Function( Event_UnlockAttempted value)?  unlockAttempted,TResult? Function( Event_WindowEnded value)?  windowEnded,TResult? Function( Event_Resync value)?  resync,}){
final _that = this;
switch (_that) {
case Event_Received() when received != null:
return received(_that);case Event_DevicesChanged() when devicesChanged != null:
return devicesChanged(_that);case Event_PairingRequested() when pairingRequested != null:
return pairingRequested(_that);case Event_PairingRequestEnded() when pairingRequestEnded != null:
return pairingRequestEnded(_that);case Event_OfferEnded() when offerEnded != null:
return offerEnded(_that);case Event_TransferRequested() when transferRequested != null:
return transferRequested(_that);case Event_TransferRequestEnded() when transferRequestEnded != null:
return transferRequestEnded(_that);case Event_TransferEnded() when transferEnded != null:
return transferEnded(_that);case Event_WindowRequested() when windowRequested != null:
return windowRequested(_that);case Event_WindowRequestEnded() when windowRequestEnded != null:
return windowRequestEnded(_that);case Event_WindowStarted() when windowStarted != null:
return windowStarted(_that);case Event_WindowPicture() when windowPicture != null:
return windowPicture(_that);case Event_WindowNotice() when windowNotice != null:
return windowNotice(_that);case Event_WindowSound() when windowSound != null:
return windowSound(_that);case Event_WindowLocked() when windowLocked != null:
return windowLocked(_that);case Event_WindowUnlock() when windowUnlock != null:
return windowUnlock(_that);case Event_UnlockAttempted() when unlockAttempted != null:
return unlockAttempted(_that);case Event_WindowEnded() when windowEnded != null:
return windowEnded(_that);case Event_Resync() when resync != null:
return resync(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( Received received)?  received,TResult Function()?  devicesChanged,TResult Function( PairingRequest request)?  pairingRequested,TResult Function( PlatformInt64 id)?  pairingRequestEnded,TResult Function( OfferOutcome outcome)?  offerEnded,TResult Function( TransferRequest request)?  transferRequested,TResult Function( PlatformInt64 id)?  transferRequestEnded,TResult Function( TransferEnd end)?  transferEnded,TResult Function( WindowRequest request)?  windowRequested,TResult Function( PlatformInt64 id,  String? failure)?  windowRequestEnded,TResult Function( WindowSession session)?  windowStarted,TResult Function( PlatformInt64 id,  int width,  int height)?  windowPicture,TResult Function( PlatformInt64 id,  WindowNoticeReason reason,  int count)?  windowNotice,TResult Function( PlatformInt64 id,  WindowSound sound)?  windowSound,TResult Function( PlatformInt64 id,  bool locked)?  windowLocked,TResult Function( PlatformInt64 id,  WindowUnlock step)?  windowUnlock,TResult Function( UnlockEntry entry)?  unlockAttempted,TResult Function( PlatformInt64 id,  WindowEnd end)?  windowEnded,TResult Function()?  resync,required TResult orElse(),}) {final _that = this;
switch (_that) {
case Event_Received() when received != null:
return received(_that.received);case Event_DevicesChanged() when devicesChanged != null:
return devicesChanged();case Event_PairingRequested() when pairingRequested != null:
return pairingRequested(_that.request);case Event_PairingRequestEnded() when pairingRequestEnded != null:
return pairingRequestEnded(_that.id);case Event_OfferEnded() when offerEnded != null:
return offerEnded(_that.outcome);case Event_TransferRequested() when transferRequested != null:
return transferRequested(_that.request);case Event_TransferRequestEnded() when transferRequestEnded != null:
return transferRequestEnded(_that.id);case Event_TransferEnded() when transferEnded != null:
return transferEnded(_that.end);case Event_WindowRequested() when windowRequested != null:
return windowRequested(_that.request);case Event_WindowRequestEnded() when windowRequestEnded != null:
return windowRequestEnded(_that.id,_that.failure);case Event_WindowStarted() when windowStarted != null:
return windowStarted(_that.session);case Event_WindowPicture() when windowPicture != null:
return windowPicture(_that.id,_that.width,_that.height);case Event_WindowNotice() when windowNotice != null:
return windowNotice(_that.id,_that.reason,_that.count);case Event_WindowSound() when windowSound != null:
return windowSound(_that.id,_that.sound);case Event_WindowLocked() when windowLocked != null:
return windowLocked(_that.id,_that.locked);case Event_WindowUnlock() when windowUnlock != null:
return windowUnlock(_that.id,_that.step);case Event_UnlockAttempted() when unlockAttempted != null:
return unlockAttempted(_that.entry);case Event_WindowEnded() when windowEnded != null:
return windowEnded(_that.id,_that.end);case Event_Resync() when resync != null:
return resync();case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( Received received)  received,required TResult Function()  devicesChanged,required TResult Function( PairingRequest request)  pairingRequested,required TResult Function( PlatformInt64 id)  pairingRequestEnded,required TResult Function( OfferOutcome outcome)  offerEnded,required TResult Function( TransferRequest request)  transferRequested,required TResult Function( PlatformInt64 id)  transferRequestEnded,required TResult Function( TransferEnd end)  transferEnded,required TResult Function( WindowRequest request)  windowRequested,required TResult Function( PlatformInt64 id,  String? failure)  windowRequestEnded,required TResult Function( WindowSession session)  windowStarted,required TResult Function( PlatformInt64 id,  int width,  int height)  windowPicture,required TResult Function( PlatformInt64 id,  WindowNoticeReason reason,  int count)  windowNotice,required TResult Function( PlatformInt64 id,  WindowSound sound)  windowSound,required TResult Function( PlatformInt64 id,  bool locked)  windowLocked,required TResult Function( PlatformInt64 id,  WindowUnlock step)  windowUnlock,required TResult Function( UnlockEntry entry)  unlockAttempted,required TResult Function( PlatformInt64 id,  WindowEnd end)  windowEnded,required TResult Function()  resync,}) {final _that = this;
switch (_that) {
case Event_Received():
return received(_that.received);case Event_DevicesChanged():
return devicesChanged();case Event_PairingRequested():
return pairingRequested(_that.request);case Event_PairingRequestEnded():
return pairingRequestEnded(_that.id);case Event_OfferEnded():
return offerEnded(_that.outcome);case Event_TransferRequested():
return transferRequested(_that.request);case Event_TransferRequestEnded():
return transferRequestEnded(_that.id);case Event_TransferEnded():
return transferEnded(_that.end);case Event_WindowRequested():
return windowRequested(_that.request);case Event_WindowRequestEnded():
return windowRequestEnded(_that.id,_that.failure);case Event_WindowStarted():
return windowStarted(_that.session);case Event_WindowPicture():
return windowPicture(_that.id,_that.width,_that.height);case Event_WindowNotice():
return windowNotice(_that.id,_that.reason,_that.count);case Event_WindowSound():
return windowSound(_that.id,_that.sound);case Event_WindowLocked():
return windowLocked(_that.id,_that.locked);case Event_WindowUnlock():
return windowUnlock(_that.id,_that.step);case Event_UnlockAttempted():
return unlockAttempted(_that.entry);case Event_WindowEnded():
return windowEnded(_that.id,_that.end);case Event_Resync():
return resync();}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( Received received)?  received,TResult? Function()?  devicesChanged,TResult? Function( PairingRequest request)?  pairingRequested,TResult? Function( PlatformInt64 id)?  pairingRequestEnded,TResult? Function( OfferOutcome outcome)?  offerEnded,TResult? Function( TransferRequest request)?  transferRequested,TResult? Function( PlatformInt64 id)?  transferRequestEnded,TResult? Function( TransferEnd end)?  transferEnded,TResult? Function( WindowRequest request)?  windowRequested,TResult? Function( PlatformInt64 id,  String? failure)?  windowRequestEnded,TResult? Function( WindowSession session)?  windowStarted,TResult? Function( PlatformInt64 id,  int width,  int height)?  windowPicture,TResult? Function( PlatformInt64 id,  WindowNoticeReason reason,  int count)?  windowNotice,TResult? Function( PlatformInt64 id,  WindowSound sound)?  windowSound,TResult? Function( PlatformInt64 id,  bool locked)?  windowLocked,TResult? Function( PlatformInt64 id,  WindowUnlock step)?  windowUnlock,TResult? Function( UnlockEntry entry)?  unlockAttempted,TResult? Function( PlatformInt64 id,  WindowEnd end)?  windowEnded,TResult? Function()?  resync,}) {final _that = this;
switch (_that) {
case Event_Received() when received != null:
return received(_that.received);case Event_DevicesChanged() when devicesChanged != null:
return devicesChanged();case Event_PairingRequested() when pairingRequested != null:
return pairingRequested(_that.request);case Event_PairingRequestEnded() when pairingRequestEnded != null:
return pairingRequestEnded(_that.id);case Event_OfferEnded() when offerEnded != null:
return offerEnded(_that.outcome);case Event_TransferRequested() when transferRequested != null:
return transferRequested(_that.request);case Event_TransferRequestEnded() when transferRequestEnded != null:
return transferRequestEnded(_that.id);case Event_TransferEnded() when transferEnded != null:
return transferEnded(_that.end);case Event_WindowRequested() when windowRequested != null:
return windowRequested(_that.request);case Event_WindowRequestEnded() when windowRequestEnded != null:
return windowRequestEnded(_that.id,_that.failure);case Event_WindowStarted() when windowStarted != null:
return windowStarted(_that.session);case Event_WindowPicture() when windowPicture != null:
return windowPicture(_that.id,_that.width,_that.height);case Event_WindowNotice() when windowNotice != null:
return windowNotice(_that.id,_that.reason,_that.count);case Event_WindowSound() when windowSound != null:
return windowSound(_that.id,_that.sound);case Event_WindowLocked() when windowLocked != null:
return windowLocked(_that.id,_that.locked);case Event_WindowUnlock() when windowUnlock != null:
return windowUnlock(_that.id,_that.step);case Event_UnlockAttempted() when unlockAttempted != null:
return unlockAttempted(_that.entry);case Event_WindowEnded() when windowEnded != null:
return windowEnded(_that.id,_that.end);case Event_Resync() when resync != null:
return resync();case _:
  return null;

}
}

}

/// @nodoc


class Event_Received extends Event {
  const Event_Received({required this.received}): super._();
  

/// What arrived.
 final  Received received;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$Event_ReceivedCopyWith<Event_Received> get copyWith => _$Event_ReceivedCopyWithImpl<Event_Received>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Event_Received&&(identical(other.received, received) || other.received == received));
}


@override
int get hashCode {
    return Object.hash(runtimeType,received);
}

@override
String toString() {
    return 'Event.received(received: $received)';
}


}

/// @nodoc
abstract mixin class $Event_ReceivedCopyWith<$Res> implements $EventCopyWith<$Res> {
  factory $Event_ReceivedCopyWith(Event_Received value, $Res Function(Event_Received) _then) = _$Event_ReceivedCopyWithImpl;
@useResult
$Res call({
 Received received
});




}
/// @nodoc
class _$Event_ReceivedCopyWithImpl<$Res>
    implements $Event_ReceivedCopyWith<$Res> {
  _$Event_ReceivedCopyWithImpl(this._self, this._then);

  final Event_Received _self;
  final $Res Function(Event_Received) _then;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? received = null,}) {
  return _then(Event_Received(
received: null == received ? _self.received : received // ignore: cast_nullable_to_non_nullable
as Received,
  ));
}


}

/// @nodoc


class Event_DevicesChanged extends Event {
  const Event_DevicesChanged(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Event_DevicesChanged);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'Event.devicesChanged()';
}


}




/// @nodoc


class Event_PairingRequested extends Event {
  const Event_PairingRequested({required this.request}): super._();
  

/// The request.
 final  PairingRequest request;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$Event_PairingRequestedCopyWith<Event_PairingRequested> get copyWith => _$Event_PairingRequestedCopyWithImpl<Event_PairingRequested>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Event_PairingRequested&&(identical(other.request, request) || other.request == request));
}


@override
int get hashCode {
    return Object.hash(runtimeType,request);
}

@override
String toString() {
    return 'Event.pairingRequested(request: $request)';
}


}

/// @nodoc
abstract mixin class $Event_PairingRequestedCopyWith<$Res> implements $EventCopyWith<$Res> {
  factory $Event_PairingRequestedCopyWith(Event_PairingRequested value, $Res Function(Event_PairingRequested) _then) = _$Event_PairingRequestedCopyWithImpl;
@useResult
$Res call({
 PairingRequest request
});




}
/// @nodoc
class _$Event_PairingRequestedCopyWithImpl<$Res>
    implements $Event_PairingRequestedCopyWith<$Res> {
  _$Event_PairingRequestedCopyWithImpl(this._self, this._then);

  final Event_PairingRequested _self;
  final $Res Function(Event_PairingRequested) _then;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? request = null,}) {
  return _then(Event_PairingRequested(
request: null == request ? _self.request : request // ignore: cast_nullable_to_non_nullable
as PairingRequest,
  ));
}


}

/// @nodoc


class Event_PairingRequestEnded extends Event {
  const Event_PairingRequestEnded({required this.id}): super._();
  

/// The `PairingRequest.id` that ended.
 final  PlatformInt64 id;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$Event_PairingRequestEndedCopyWith<Event_PairingRequestEnded> get copyWith => _$Event_PairingRequestEndedCopyWithImpl<Event_PairingRequestEnded>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Event_PairingRequestEnded&&(identical(other.id, id) || other.id == id));
}


@override
int get hashCode {
    return Object.hash(runtimeType,id);
}

@override
String toString() {
    return 'Event.pairingRequestEnded(id: $id)';
}


}

/// @nodoc
abstract mixin class $Event_PairingRequestEndedCopyWith<$Res> implements $EventCopyWith<$Res> {
  factory $Event_PairingRequestEndedCopyWith(Event_PairingRequestEnded value, $Res Function(Event_PairingRequestEnded) _then) = _$Event_PairingRequestEndedCopyWithImpl;
@useResult
$Res call({
 PlatformInt64 id
});




}
/// @nodoc
class _$Event_PairingRequestEndedCopyWithImpl<$Res>
    implements $Event_PairingRequestEndedCopyWith<$Res> {
  _$Event_PairingRequestEndedCopyWithImpl(this._self, this._then);

  final Event_PairingRequestEnded _self;
  final $Res Function(Event_PairingRequestEnded) _then;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? id = null,}) {
  return _then(Event_PairingRequestEnded(
id: null == id ? _self.id : id // ignore: cast_nullable_to_non_nullable
as PlatformInt64,
  ));
}


}

/// @nodoc


class Event_OfferEnded extends Event {
  const Event_OfferEnded({required this.outcome}): super._();
  

/// How.
 final  OfferOutcome outcome;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$Event_OfferEndedCopyWith<Event_OfferEnded> get copyWith => _$Event_OfferEndedCopyWithImpl<Event_OfferEnded>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Event_OfferEnded&&(identical(other.outcome, outcome) || other.outcome == outcome));
}


@override
int get hashCode {
    return Object.hash(runtimeType,outcome);
}

@override
String toString() {
    return 'Event.offerEnded(outcome: $outcome)';
}


}

/// @nodoc
abstract mixin class $Event_OfferEndedCopyWith<$Res> implements $EventCopyWith<$Res> {
  factory $Event_OfferEndedCopyWith(Event_OfferEnded value, $Res Function(Event_OfferEnded) _then) = _$Event_OfferEndedCopyWithImpl;
@useResult
$Res call({
 OfferOutcome outcome
});


$OfferOutcomeCopyWith<$Res> get outcome;

}
/// @nodoc
class _$Event_OfferEndedCopyWithImpl<$Res>
    implements $Event_OfferEndedCopyWith<$Res> {
  _$Event_OfferEndedCopyWithImpl(this._self, this._then);

  final Event_OfferEnded _self;
  final $Res Function(Event_OfferEnded) _then;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? outcome = null,}) {
  return _then(Event_OfferEnded(
outcome: null == outcome ? _self.outcome : outcome // ignore: cast_nullable_to_non_nullable
as OfferOutcome,
  ));
}

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@override
@pragma('vm:prefer-inline')
$OfferOutcomeCopyWith<$Res> get outcome {
  
  return $OfferOutcomeCopyWith<$Res>(_self.outcome, (value) {
    return _then(_self.copyWith(outcome: value));
  });
}
}

/// @nodoc


class Event_TransferRequested extends Event {
  const Event_TransferRequested({required this.request}): super._();
  

/// The request.
 final  TransferRequest request;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$Event_TransferRequestedCopyWith<Event_TransferRequested> get copyWith => _$Event_TransferRequestedCopyWithImpl<Event_TransferRequested>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Event_TransferRequested&&(identical(other.request, request) || other.request == request));
}


@override
int get hashCode {
    return Object.hash(runtimeType,request);
}

@override
String toString() {
    return 'Event.transferRequested(request: $request)';
}


}

/// @nodoc
abstract mixin class $Event_TransferRequestedCopyWith<$Res> implements $EventCopyWith<$Res> {
  factory $Event_TransferRequestedCopyWith(Event_TransferRequested value, $Res Function(Event_TransferRequested) _then) = _$Event_TransferRequestedCopyWithImpl;
@useResult
$Res call({
 TransferRequest request
});




}
/// @nodoc
class _$Event_TransferRequestedCopyWithImpl<$Res>
    implements $Event_TransferRequestedCopyWith<$Res> {
  _$Event_TransferRequestedCopyWithImpl(this._self, this._then);

  final Event_TransferRequested _self;
  final $Res Function(Event_TransferRequested) _then;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? request = null,}) {
  return _then(Event_TransferRequested(
request: null == request ? _self.request : request // ignore: cast_nullable_to_non_nullable
as TransferRequest,
  ));
}


}

/// @nodoc


class Event_TransferRequestEnded extends Event {
  const Event_TransferRequestEnded({required this.id}): super._();
  

/// The `TransferRequest.id` that ended.
 final  PlatformInt64 id;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$Event_TransferRequestEndedCopyWith<Event_TransferRequestEnded> get copyWith => _$Event_TransferRequestEndedCopyWithImpl<Event_TransferRequestEnded>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Event_TransferRequestEnded&&(identical(other.id, id) || other.id == id));
}


@override
int get hashCode {
    return Object.hash(runtimeType,id);
}

@override
String toString() {
    return 'Event.transferRequestEnded(id: $id)';
}


}

/// @nodoc
abstract mixin class $Event_TransferRequestEndedCopyWith<$Res> implements $EventCopyWith<$Res> {
  factory $Event_TransferRequestEndedCopyWith(Event_TransferRequestEnded value, $Res Function(Event_TransferRequestEnded) _then) = _$Event_TransferRequestEndedCopyWithImpl;
@useResult
$Res call({
 PlatformInt64 id
});




}
/// @nodoc
class _$Event_TransferRequestEndedCopyWithImpl<$Res>
    implements $Event_TransferRequestEndedCopyWith<$Res> {
  _$Event_TransferRequestEndedCopyWithImpl(this._self, this._then);

  final Event_TransferRequestEnded _self;
  final $Res Function(Event_TransferRequestEnded) _then;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? id = null,}) {
  return _then(Event_TransferRequestEnded(
id: null == id ? _self.id : id // ignore: cast_nullable_to_non_nullable
as PlatformInt64,
  ));
}


}

/// @nodoc


class Event_TransferEnded extends Event {
  const Event_TransferEnded({required this.end}): super._();
  

/// How it ended.
 final  TransferEnd end;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$Event_TransferEndedCopyWith<Event_TransferEnded> get copyWith => _$Event_TransferEndedCopyWithImpl<Event_TransferEnded>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Event_TransferEnded&&(identical(other.end, end) || other.end == end));
}


@override
int get hashCode {
    return Object.hash(runtimeType,end);
}

@override
String toString() {
    return 'Event.transferEnded(end: $end)';
}


}

/// @nodoc
abstract mixin class $Event_TransferEndedCopyWith<$Res> implements $EventCopyWith<$Res> {
  factory $Event_TransferEndedCopyWith(Event_TransferEnded value, $Res Function(Event_TransferEnded) _then) = _$Event_TransferEndedCopyWithImpl;
@useResult
$Res call({
 TransferEnd end
});




}
/// @nodoc
class _$Event_TransferEndedCopyWithImpl<$Res>
    implements $Event_TransferEndedCopyWith<$Res> {
  _$Event_TransferEndedCopyWithImpl(this._self, this._then);

  final Event_TransferEnded _self;
  final $Res Function(Event_TransferEnded) _then;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? end = null,}) {
  return _then(Event_TransferEnded(
end: null == end ? _self.end : end // ignore: cast_nullable_to_non_nullable
as TransferEnd,
  ));
}


}

/// @nodoc


class Event_WindowRequested extends Event {
  const Event_WindowRequested({required this.request}): super._();
  

/// The request.
 final  WindowRequest request;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$Event_WindowRequestedCopyWith<Event_WindowRequested> get copyWith => _$Event_WindowRequestedCopyWithImpl<Event_WindowRequested>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Event_WindowRequested&&(identical(other.request, request) || other.request == request));
}


@override
int get hashCode {
    return Object.hash(runtimeType,request);
}

@override
String toString() {
    return 'Event.windowRequested(request: $request)';
}


}

/// @nodoc
abstract mixin class $Event_WindowRequestedCopyWith<$Res> implements $EventCopyWith<$Res> {
  factory $Event_WindowRequestedCopyWith(Event_WindowRequested value, $Res Function(Event_WindowRequested) _then) = _$Event_WindowRequestedCopyWithImpl;
@useResult
$Res call({
 WindowRequest request
});




}
/// @nodoc
class _$Event_WindowRequestedCopyWithImpl<$Res>
    implements $Event_WindowRequestedCopyWith<$Res> {
  _$Event_WindowRequestedCopyWithImpl(this._self, this._then);

  final Event_WindowRequested _self;
  final $Res Function(Event_WindowRequested) _then;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? request = null,}) {
  return _then(Event_WindowRequested(
request: null == request ? _self.request : request // ignore: cast_nullable_to_non_nullable
as WindowRequest,
  ));
}


}

/// @nodoc


class Event_WindowRequestEnded extends Event {
  const Event_WindowRequestEnded({required this.id, this.failure}): super._();
  

/// The `WindowRequest.id` that ended.
 final  PlatformInt64 id;
/// One sentence to show when the person said yes and this device could not show the
/// window after all; `null` otherwise.
 final  String? failure;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$Event_WindowRequestEndedCopyWith<Event_WindowRequestEnded> get copyWith => _$Event_WindowRequestEndedCopyWithImpl<Event_WindowRequestEnded>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Event_WindowRequestEnded&&(identical(other.id, id) || other.id == id)&&(identical(other.failure, failure) || other.failure == failure));
}


@override
int get hashCode {
    return Object.hash(runtimeType,id,failure);
}

@override
String toString() {
    return 'Event.windowRequestEnded(id: $id, failure: $failure)';
}


}

/// @nodoc
abstract mixin class $Event_WindowRequestEndedCopyWith<$Res> implements $EventCopyWith<$Res> {
  factory $Event_WindowRequestEndedCopyWith(Event_WindowRequestEnded value, $Res Function(Event_WindowRequestEnded) _then) = _$Event_WindowRequestEndedCopyWithImpl;
@useResult
$Res call({
 PlatformInt64 id, String? failure
});




}
/// @nodoc
class _$Event_WindowRequestEndedCopyWithImpl<$Res>
    implements $Event_WindowRequestEndedCopyWith<$Res> {
  _$Event_WindowRequestEndedCopyWithImpl(this._self, this._then);

  final Event_WindowRequestEnded _self;
  final $Res Function(Event_WindowRequestEnded) _then;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? id = null,Object? failure = freezed,}) {
  return _then(Event_WindowRequestEnded(
id: null == id ? _self.id : id // ignore: cast_nullable_to_non_nullable
as PlatformInt64,failure: freezed == failure ? _self.failure : failure // ignore: cast_nullable_to_non_nullable
as String?,
  ));
}


}

/// @nodoc


class Event_WindowStarted extends Event {
  const Event_WindowStarted({required this.session}): super._();
  

/// The session.
 final  WindowSession session;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$Event_WindowStartedCopyWith<Event_WindowStarted> get copyWith => _$Event_WindowStartedCopyWithImpl<Event_WindowStarted>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Event_WindowStarted&&(identical(other.session, session) || other.session == session));
}


@override
int get hashCode {
    return Object.hash(runtimeType,session);
}

@override
String toString() {
    return 'Event.windowStarted(session: $session)';
}


}

/// @nodoc
abstract mixin class $Event_WindowStartedCopyWith<$Res> implements $EventCopyWith<$Res> {
  factory $Event_WindowStartedCopyWith(Event_WindowStarted value, $Res Function(Event_WindowStarted) _then) = _$Event_WindowStartedCopyWithImpl;
@useResult
$Res call({
 WindowSession session
});




}
/// @nodoc
class _$Event_WindowStartedCopyWithImpl<$Res>
    implements $Event_WindowStartedCopyWith<$Res> {
  _$Event_WindowStartedCopyWithImpl(this._self, this._then);

  final Event_WindowStarted _self;
  final $Res Function(Event_WindowStarted) _then;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? session = null,}) {
  return _then(Event_WindowStarted(
session: null == session ? _self.session : session // ignore: cast_nullable_to_non_nullable
as WindowSession,
  ));
}


}

/// @nodoc


class Event_WindowPicture extends Event {
  const Event_WindowPicture({required this.id, required this.width, required this.height}): super._();
  

/// The `WindowSession.id` whose picture this is.
 final  PlatformInt64 id;
/// The width of the picture in pixels, at least 1.
 final  int width;
/// The height of the picture in pixels, at least 1.
 final  int height;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$Event_WindowPictureCopyWith<Event_WindowPicture> get copyWith => _$Event_WindowPictureCopyWithImpl<Event_WindowPicture>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Event_WindowPicture&&(identical(other.id, id) || other.id == id)&&(identical(other.width, width) || other.width == width)&&(identical(other.height, height) || other.height == height));
}


@override
int get hashCode {
    return Object.hash(runtimeType,id,width,height);
}

@override
String toString() {
    return 'Event.windowPicture(id: $id, width: $width, height: $height)';
}


}

/// @nodoc
abstract mixin class $Event_WindowPictureCopyWith<$Res> implements $EventCopyWith<$Res> {
  factory $Event_WindowPictureCopyWith(Event_WindowPicture value, $Res Function(Event_WindowPicture) _then) = _$Event_WindowPictureCopyWithImpl;
@useResult
$Res call({
 PlatformInt64 id, int width, int height
});




}
/// @nodoc
class _$Event_WindowPictureCopyWithImpl<$Res>
    implements $Event_WindowPictureCopyWith<$Res> {
  _$Event_WindowPictureCopyWithImpl(this._self, this._then);

  final Event_WindowPicture _self;
  final $Res Function(Event_WindowPicture) _then;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? id = null,Object? width = null,Object? height = null,}) {
  return _then(Event_WindowPicture(
id: null == id ? _self.id : id // ignore: cast_nullable_to_non_nullable
as PlatformInt64,width: null == width ? _self.width : width // ignore: cast_nullable_to_non_nullable
as int,height: null == height ? _self.height : height // ignore: cast_nullable_to_non_nullable
as int,
  ));
}


}

/// @nodoc


class Event_WindowNotice extends Event {
  const Event_WindowNotice({required this.id, required this.reason, required this.count}): super._();
  

/// The `WindowSession.id` whose events were dropped.
 final  PlatformInt64 id;
/// The rule that dropped the most of them.
 final  WindowNoticeReason reason;
/// How many events it dropped, at least 1.
 final  int count;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$Event_WindowNoticeCopyWith<Event_WindowNotice> get copyWith => _$Event_WindowNoticeCopyWithImpl<Event_WindowNotice>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Event_WindowNotice&&(identical(other.id, id) || other.id == id)&&(identical(other.reason, reason) || other.reason == reason)&&(identical(other.count, count) || other.count == count));
}


@override
int get hashCode {
    return Object.hash(runtimeType,id,reason,count);
}

@override
String toString() {
    return 'Event.windowNotice(id: $id, reason: $reason, count: $count)';
}


}

/// @nodoc
abstract mixin class $Event_WindowNoticeCopyWith<$Res> implements $EventCopyWith<$Res> {
  factory $Event_WindowNoticeCopyWith(Event_WindowNotice value, $Res Function(Event_WindowNotice) _then) = _$Event_WindowNoticeCopyWithImpl;
@useResult
$Res call({
 PlatformInt64 id, WindowNoticeReason reason, int count
});




}
/// @nodoc
class _$Event_WindowNoticeCopyWithImpl<$Res>
    implements $Event_WindowNoticeCopyWith<$Res> {
  _$Event_WindowNoticeCopyWithImpl(this._self, this._then);

  final Event_WindowNotice _self;
  final $Res Function(Event_WindowNotice) _then;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? id = null,Object? reason = null,Object? count = null,}) {
  return _then(Event_WindowNotice(
id: null == id ? _self.id : id // ignore: cast_nullable_to_non_nullable
as PlatformInt64,reason: null == reason ? _self.reason : reason // ignore: cast_nullable_to_non_nullable
as WindowNoticeReason,count: null == count ? _self.count : count // ignore: cast_nullable_to_non_nullable
as int,
  ));
}


}

/// @nodoc


class Event_WindowSound extends Event {
  const Event_WindowSound({required this.id, required this.sound}): super._();
  

/// The `WindowSession.id` whose sound this is.
 final  PlatformInt64 id;
/// How the sound stands now.
 final  WindowSound sound;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$Event_WindowSoundCopyWith<Event_WindowSound> get copyWith => _$Event_WindowSoundCopyWithImpl<Event_WindowSound>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Event_WindowSound&&(identical(other.id, id) || other.id == id)&&(identical(other.sound, sound) || other.sound == sound));
}


@override
int get hashCode {
    return Object.hash(runtimeType,id,sound);
}

@override
String toString() {
    return 'Event.windowSound(id: $id, sound: $sound)';
}


}

/// @nodoc
abstract mixin class $Event_WindowSoundCopyWith<$Res> implements $EventCopyWith<$Res> {
  factory $Event_WindowSoundCopyWith(Event_WindowSound value, $Res Function(Event_WindowSound) _then) = _$Event_WindowSoundCopyWithImpl;
@useResult
$Res call({
 PlatformInt64 id, WindowSound sound
});




}
/// @nodoc
class _$Event_WindowSoundCopyWithImpl<$Res>
    implements $Event_WindowSoundCopyWith<$Res> {
  _$Event_WindowSoundCopyWithImpl(this._self, this._then);

  final Event_WindowSound _self;
  final $Res Function(Event_WindowSound) _then;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? id = null,Object? sound = null,}) {
  return _then(Event_WindowSound(
id: null == id ? _self.id : id // ignore: cast_nullable_to_non_nullable
as PlatformInt64,sound: null == sound ? _self.sound : sound // ignore: cast_nullable_to_non_nullable
as WindowSound,
  ));
}


}

/// @nodoc


class Event_WindowLocked extends Event {
  const Event_WindowLocked({required this.id, required this.locked}): super._();
  

/// The `WindowSession.id` whose screen this is.
 final  PlatformInt64 id;
/// Whether it is locked now.
 final  bool locked;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$Event_WindowLockedCopyWith<Event_WindowLocked> get copyWith => _$Event_WindowLockedCopyWithImpl<Event_WindowLocked>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Event_WindowLocked&&(identical(other.id, id) || other.id == id)&&(identical(other.locked, locked) || other.locked == locked));
}


@override
int get hashCode {
    return Object.hash(runtimeType,id,locked);
}

@override
String toString() {
    return 'Event.windowLocked(id: $id, locked: $locked)';
}


}

/// @nodoc
abstract mixin class $Event_WindowLockedCopyWith<$Res> implements $EventCopyWith<$Res> {
  factory $Event_WindowLockedCopyWith(Event_WindowLocked value, $Res Function(Event_WindowLocked) _then) = _$Event_WindowLockedCopyWithImpl;
@useResult
$Res call({
 PlatformInt64 id, bool locked
});




}
/// @nodoc
class _$Event_WindowLockedCopyWithImpl<$Res>
    implements $Event_WindowLockedCopyWith<$Res> {
  _$Event_WindowLockedCopyWithImpl(this._self, this._then);

  final Event_WindowLocked _self;
  final $Res Function(Event_WindowLocked) _then;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? id = null,Object? locked = null,}) {
  return _then(Event_WindowLocked(
id: null == id ? _self.id : id // ignore: cast_nullable_to_non_nullable
as PlatformInt64,locked: null == locked ? _self.locked : locked // ignore: cast_nullable_to_non_nullable
as bool,
  ));
}


}

/// @nodoc


class Event_WindowUnlock extends Event {
  const Event_WindowUnlock({required this.id, required this.step}): super._();
  

/// The `WindowSession.id` whose screen this is.
 final  PlatformInt64 id;
/// How the attempt stands now.
 final  WindowUnlock step;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$Event_WindowUnlockCopyWith<Event_WindowUnlock> get copyWith => _$Event_WindowUnlockCopyWithImpl<Event_WindowUnlock>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Event_WindowUnlock&&(identical(other.id, id) || other.id == id)&&(identical(other.step, step) || other.step == step));
}


@override
int get hashCode {
    return Object.hash(runtimeType,id,step);
}

@override
String toString() {
    return 'Event.windowUnlock(id: $id, step: $step)';
}


}

/// @nodoc
abstract mixin class $Event_WindowUnlockCopyWith<$Res> implements $EventCopyWith<$Res> {
  factory $Event_WindowUnlockCopyWith(Event_WindowUnlock value, $Res Function(Event_WindowUnlock) _then) = _$Event_WindowUnlockCopyWithImpl;
@useResult
$Res call({
 PlatformInt64 id, WindowUnlock step
});


$WindowUnlockCopyWith<$Res> get step;

}
/// @nodoc
class _$Event_WindowUnlockCopyWithImpl<$Res>
    implements $Event_WindowUnlockCopyWith<$Res> {
  _$Event_WindowUnlockCopyWithImpl(this._self, this._then);

  final Event_WindowUnlock _self;
  final $Res Function(Event_WindowUnlock) _then;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? id = null,Object? step = null,}) {
  return _then(Event_WindowUnlock(
id: null == id ? _self.id : id // ignore: cast_nullable_to_non_nullable
as PlatformInt64,step: null == step ? _self.step : step // ignore: cast_nullable_to_non_nullable
as WindowUnlock,
  ));
}

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@override
@pragma('vm:prefer-inline')
$WindowUnlockCopyWith<$Res> get step {
  
  return $WindowUnlockCopyWith<$Res>(_self.step, (value) {
    return _then(_self.copyWith(step: value));
  });
}
}

/// @nodoc


class Event_UnlockAttempted extends Event {
  const Event_UnlockAttempted({required this.entry}): super._();
  

/// The attempt.
 final  UnlockEntry entry;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$Event_UnlockAttemptedCopyWith<Event_UnlockAttempted> get copyWith => _$Event_UnlockAttemptedCopyWithImpl<Event_UnlockAttempted>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Event_UnlockAttempted&&(identical(other.entry, entry) || other.entry == entry));
}


@override
int get hashCode {
    return Object.hash(runtimeType,entry);
}

@override
String toString() {
    return 'Event.unlockAttempted(entry: $entry)';
}


}

/// @nodoc
abstract mixin class $Event_UnlockAttemptedCopyWith<$Res> implements $EventCopyWith<$Res> {
  factory $Event_UnlockAttemptedCopyWith(Event_UnlockAttempted value, $Res Function(Event_UnlockAttempted) _then) = _$Event_UnlockAttemptedCopyWithImpl;
@useResult
$Res call({
 UnlockEntry entry
});




}
/// @nodoc
class _$Event_UnlockAttemptedCopyWithImpl<$Res>
    implements $Event_UnlockAttemptedCopyWith<$Res> {
  _$Event_UnlockAttemptedCopyWithImpl(this._self, this._then);

  final Event_UnlockAttempted _self;
  final $Res Function(Event_UnlockAttempted) _then;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? entry = null,}) {
  return _then(Event_UnlockAttempted(
entry: null == entry ? _self.entry : entry // ignore: cast_nullable_to_non_nullable
as UnlockEntry,
  ));
}


}

/// @nodoc


class Event_WindowEnded extends Event {
  const Event_WindowEnded({required this.id, required this.end}): super._();
  

/// The `WindowSession.id` that ended.
 final  PlatformInt64 id;
/// How.
 final  WindowEnd end;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$Event_WindowEndedCopyWith<Event_WindowEnded> get copyWith => _$Event_WindowEndedCopyWithImpl<Event_WindowEnded>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Event_WindowEnded&&(identical(other.id, id) || other.id == id)&&(identical(other.end, end) || other.end == end));
}


@override
int get hashCode {
    return Object.hash(runtimeType,id,end);
}

@override
String toString() {
    return 'Event.windowEnded(id: $id, end: $end)';
}


}

/// @nodoc
abstract mixin class $Event_WindowEndedCopyWith<$Res> implements $EventCopyWith<$Res> {
  factory $Event_WindowEndedCopyWith(Event_WindowEnded value, $Res Function(Event_WindowEnded) _then) = _$Event_WindowEndedCopyWithImpl;
@useResult
$Res call({
 PlatformInt64 id, WindowEnd end
});


$WindowEndCopyWith<$Res> get end;

}
/// @nodoc
class _$Event_WindowEndedCopyWithImpl<$Res>
    implements $Event_WindowEndedCopyWith<$Res> {
  _$Event_WindowEndedCopyWithImpl(this._self, this._then);

  final Event_WindowEnded _self;
  final $Res Function(Event_WindowEnded) _then;

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? id = null,Object? end = null,}) {
  return _then(Event_WindowEnded(
id: null == id ? _self.id : id // ignore: cast_nullable_to_non_nullable
as PlatformInt64,end: null == end ? _self.end : end // ignore: cast_nullable_to_non_nullable
as WindowEnd,
  ));
}

/// Create a copy of Event
/// with the given fields replaced by the non-null parameter values.
@override
@pragma('vm:prefer-inline')
$WindowEndCopyWith<$Res> get end {
  
  return $WindowEndCopyWith<$Res>(_self.end, (value) {
    return _then(_self.copyWith(end: value));
  });
}
}

/// @nodoc


class Event_Resync extends Event {
  const Event_Resync(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Event_Resync);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'Event.resync()';
}


}




/// @nodoc
mixin _$JoinProgress {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is JoinProgress);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'JoinProgress()';
}


}

/// @nodoc
class $JoinProgressCopyWith<$Res>  {
$JoinProgressCopyWith(JoinProgress _, $Res Function(JoinProgress) __);
}


/// Adds pattern-matching-related methods to [JoinProgress].
extension JoinProgressPatterns on JoinProgress {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( JoinProgress_Code value)?  code,TResult Function( JoinProgress_Paired value)?  paired,TResult Function( JoinProgress_Failed value)?  failed,required TResult orElse(),}){
final _that = this;
switch (_that) {
case JoinProgress_Code() when code != null:
return code(_that);case JoinProgress_Paired() when paired != null:
return paired(_that);case JoinProgress_Failed() when failed != null:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( JoinProgress_Code value)  code,required TResult Function( JoinProgress_Paired value)  paired,required TResult Function( JoinProgress_Failed value)  failed,}){
final _that = this;
switch (_that) {
case JoinProgress_Code():
return code(_that);case JoinProgress_Paired():
return paired(_that);case JoinProgress_Failed():
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( JoinProgress_Code value)?  code,TResult? Function( JoinProgress_Paired value)?  paired,TResult? Function( JoinProgress_Failed value)?  failed,}){
final _that = this;
switch (_that) {
case JoinProgress_Code() when code != null:
return code(_that);case JoinProgress_Paired() when paired != null:
return paired(_that);case JoinProgress_Failed() when failed != null:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( String code)?  code,TResult Function( Device device)?  paired,TResult Function( BatonError error)?  failed,required TResult orElse(),}) {final _that = this;
switch (_that) {
case JoinProgress_Code() when code != null:
return code(_that.code);case JoinProgress_Paired() when paired != null:
return paired(_that.device);case JoinProgress_Failed() when failed != null:
return failed(_that.error);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( String code)  code,required TResult Function( Device device)  paired,required TResult Function( BatonError error)  failed,}) {final _that = this;
switch (_that) {
case JoinProgress_Code():
return code(_that.code);case JoinProgress_Paired():
return paired(_that.device);case JoinProgress_Failed():
return failed(_that.error);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( String code)?  code,TResult? Function( Device device)?  paired,TResult? Function( BatonError error)?  failed,}) {final _that = this;
switch (_that) {
case JoinProgress_Code() when code != null:
return code(_that.code);case JoinProgress_Paired() when paired != null:
return paired(_that.device);case JoinProgress_Failed() when failed != null:
return failed(_that.error);case _:
  return null;

}
}

}

/// @nodoc


class JoinProgress_Code extends JoinProgress {
  const JoinProgress_Code({required this.code}): super._();
  

/// Six digits, as `514 643`.
 final  String code;

/// Create a copy of JoinProgress
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$JoinProgress_CodeCopyWith<JoinProgress_Code> get copyWith => _$JoinProgress_CodeCopyWithImpl<JoinProgress_Code>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is JoinProgress_Code&&(identical(other.code, code) || other.code == code));
}


@override
int get hashCode {
    return Object.hash(runtimeType,code);
}

@override
String toString() {
    return 'JoinProgress.code(code: $code)';
}


}

/// @nodoc
abstract mixin class $JoinProgress_CodeCopyWith<$Res> implements $JoinProgressCopyWith<$Res> {
  factory $JoinProgress_CodeCopyWith(JoinProgress_Code value, $Res Function(JoinProgress_Code) _then) = _$JoinProgress_CodeCopyWithImpl;
@useResult
$Res call({
 String code
});




}
/// @nodoc
class _$JoinProgress_CodeCopyWithImpl<$Res>
    implements $JoinProgress_CodeCopyWith<$Res> {
  _$JoinProgress_CodeCopyWithImpl(this._self, this._then);

  final JoinProgress_Code _self;
  final $Res Function(JoinProgress_Code) _then;

/// Create a copy of JoinProgress
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? code = null,}) {
  return _then(JoinProgress_Code(
code: null == code ? _self.code : code // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class JoinProgress_Paired extends JoinProgress {
  const JoinProgress_Paired({required this.device}): super._();
  

/// The device that is now paired.
 final  Device device;

/// Create a copy of JoinProgress
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$JoinProgress_PairedCopyWith<JoinProgress_Paired> get copyWith => _$JoinProgress_PairedCopyWithImpl<JoinProgress_Paired>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is JoinProgress_Paired&&(identical(other.device, device) || other.device == device));
}


@override
int get hashCode {
    return Object.hash(runtimeType,device);
}

@override
String toString() {
    return 'JoinProgress.paired(device: $device)';
}


}

/// @nodoc
abstract mixin class $JoinProgress_PairedCopyWith<$Res> implements $JoinProgressCopyWith<$Res> {
  factory $JoinProgress_PairedCopyWith(JoinProgress_Paired value, $Res Function(JoinProgress_Paired) _then) = _$JoinProgress_PairedCopyWithImpl;
@useResult
$Res call({
 Device device
});




}
/// @nodoc
class _$JoinProgress_PairedCopyWithImpl<$Res>
    implements $JoinProgress_PairedCopyWith<$Res> {
  _$JoinProgress_PairedCopyWithImpl(this._self, this._then);

  final JoinProgress_Paired _self;
  final $Res Function(JoinProgress_Paired) _then;

/// Create a copy of JoinProgress
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? device = null,}) {
  return _then(JoinProgress_Paired(
device: null == device ? _self.device : device // ignore: cast_nullable_to_non_nullable
as Device,
  ));
}


}

/// @nodoc


class JoinProgress_Failed extends JoinProgress {
  const JoinProgress_Failed({required this.error}): super._();
  

/// Why.
 final  BatonError error;

/// Create a copy of JoinProgress
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$JoinProgress_FailedCopyWith<JoinProgress_Failed> get copyWith => _$JoinProgress_FailedCopyWithImpl<JoinProgress_Failed>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is JoinProgress_Failed&&(identical(other.error, error) || other.error == error));
}


@override
int get hashCode {
    return Object.hash(runtimeType,error);
}

@override
String toString() {
    return 'JoinProgress.failed(error: $error)';
}


}

/// @nodoc
abstract mixin class $JoinProgress_FailedCopyWith<$Res> implements $JoinProgressCopyWith<$Res> {
  factory $JoinProgress_FailedCopyWith(JoinProgress_Failed value, $Res Function(JoinProgress_Failed) _then) = _$JoinProgress_FailedCopyWithImpl;
@useResult
$Res call({
 BatonError error
});




}
/// @nodoc
class _$JoinProgress_FailedCopyWithImpl<$Res>
    implements $JoinProgress_FailedCopyWith<$Res> {
  _$JoinProgress_FailedCopyWithImpl(this._self, this._then);

  final JoinProgress_Failed _self;
  final $Res Function(JoinProgress_Failed) _then;

/// Create a copy of JoinProgress
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? error = null,}) {
  return _then(JoinProgress_Failed(
error: null == error ? _self.error : error // ignore: cast_nullable_to_non_nullable
as BatonError,
  ));
}


}

/// @nodoc
mixin _$OfferOutcome {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is OfferOutcome);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'OfferOutcome()';
}


}

/// @nodoc
class $OfferOutcomeCopyWith<$Res>  {
$OfferOutcomeCopyWith(OfferOutcome _, $Res Function(OfferOutcome) __);
}


/// Adds pattern-matching-related methods to [OfferOutcome].
extension OfferOutcomePatterns on OfferOutcome {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( OfferOutcome_Paired value)?  paired,TResult Function( OfferOutcome_Expired value)?  expired,TResult Function( OfferOutcome_Cancelled value)?  cancelled,TResult Function( OfferOutcome_Declined value)?  declined,TResult Function( OfferOutcome_Abandoned value)?  abandoned,TResult Function( OfferOutcome_Failed value)?  failed,required TResult orElse(),}){
final _that = this;
switch (_that) {
case OfferOutcome_Paired() when paired != null:
return paired(_that);case OfferOutcome_Expired() when expired != null:
return expired(_that);case OfferOutcome_Cancelled() when cancelled != null:
return cancelled(_that);case OfferOutcome_Declined() when declined != null:
return declined(_that);case OfferOutcome_Abandoned() when abandoned != null:
return abandoned(_that);case OfferOutcome_Failed() when failed != null:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( OfferOutcome_Paired value)  paired,required TResult Function( OfferOutcome_Expired value)  expired,required TResult Function( OfferOutcome_Cancelled value)  cancelled,required TResult Function( OfferOutcome_Declined value)  declined,required TResult Function( OfferOutcome_Abandoned value)  abandoned,required TResult Function( OfferOutcome_Failed value)  failed,}){
final _that = this;
switch (_that) {
case OfferOutcome_Paired():
return paired(_that);case OfferOutcome_Expired():
return expired(_that);case OfferOutcome_Cancelled():
return cancelled(_that);case OfferOutcome_Declined():
return declined(_that);case OfferOutcome_Abandoned():
return abandoned(_that);case OfferOutcome_Failed():
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( OfferOutcome_Paired value)?  paired,TResult? Function( OfferOutcome_Expired value)?  expired,TResult? Function( OfferOutcome_Cancelled value)?  cancelled,TResult? Function( OfferOutcome_Declined value)?  declined,TResult? Function( OfferOutcome_Abandoned value)?  abandoned,TResult? Function( OfferOutcome_Failed value)?  failed,}){
final _that = this;
switch (_that) {
case OfferOutcome_Paired() when paired != null:
return paired(_that);case OfferOutcome_Expired() when expired != null:
return expired(_that);case OfferOutcome_Cancelled() when cancelled != null:
return cancelled(_that);case OfferOutcome_Declined() when declined != null:
return declined(_that);case OfferOutcome_Abandoned() when abandoned != null:
return abandoned(_that);case OfferOutcome_Failed() when failed != null:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( Device device)?  paired,TResult Function()?  expired,TResult Function()?  cancelled,TResult Function()?  declined,TResult Function()?  abandoned,TResult Function( String message)?  failed,required TResult orElse(),}) {final _that = this;
switch (_that) {
case OfferOutcome_Paired() when paired != null:
return paired(_that.device);case OfferOutcome_Expired() when expired != null:
return expired();case OfferOutcome_Cancelled() when cancelled != null:
return cancelled();case OfferOutcome_Declined() when declined != null:
return declined();case OfferOutcome_Abandoned() when abandoned != null:
return abandoned();case OfferOutcome_Failed() when failed != null:
return failed(_that.message);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( Device device)  paired,required TResult Function()  expired,required TResult Function()  cancelled,required TResult Function()  declined,required TResult Function()  abandoned,required TResult Function( String message)  failed,}) {final _that = this;
switch (_that) {
case OfferOutcome_Paired():
return paired(_that.device);case OfferOutcome_Expired():
return expired();case OfferOutcome_Cancelled():
return cancelled();case OfferOutcome_Declined():
return declined();case OfferOutcome_Abandoned():
return abandoned();case OfferOutcome_Failed():
return failed(_that.message);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( Device device)?  paired,TResult? Function()?  expired,TResult? Function()?  cancelled,TResult? Function()?  declined,TResult? Function()?  abandoned,TResult? Function( String message)?  failed,}) {final _that = this;
switch (_that) {
case OfferOutcome_Paired() when paired != null:
return paired(_that.device);case OfferOutcome_Expired() when expired != null:
return expired();case OfferOutcome_Cancelled() when cancelled != null:
return cancelled();case OfferOutcome_Declined() when declined != null:
return declined();case OfferOutcome_Abandoned() when abandoned != null:
return abandoned();case OfferOutcome_Failed() when failed != null:
return failed(_that.message);case _:
  return null;

}
}

}

/// @nodoc


class OfferOutcome_Paired extends OfferOutcome {
  const OfferOutcome_Paired({required this.device}): super._();
  

/// The device that is now paired.
 final  Device device;

/// Create a copy of OfferOutcome
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$OfferOutcome_PairedCopyWith<OfferOutcome_Paired> get copyWith => _$OfferOutcome_PairedCopyWithImpl<OfferOutcome_Paired>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is OfferOutcome_Paired&&(identical(other.device, device) || other.device == device));
}


@override
int get hashCode {
    return Object.hash(runtimeType,device);
}

@override
String toString() {
    return 'OfferOutcome.paired(device: $device)';
}


}

/// @nodoc
abstract mixin class $OfferOutcome_PairedCopyWith<$Res> implements $OfferOutcomeCopyWith<$Res> {
  factory $OfferOutcome_PairedCopyWith(OfferOutcome_Paired value, $Res Function(OfferOutcome_Paired) _then) = _$OfferOutcome_PairedCopyWithImpl;
@useResult
$Res call({
 Device device
});




}
/// @nodoc
class _$OfferOutcome_PairedCopyWithImpl<$Res>
    implements $OfferOutcome_PairedCopyWith<$Res> {
  _$OfferOutcome_PairedCopyWithImpl(this._self, this._then);

  final OfferOutcome_Paired _self;
  final $Res Function(OfferOutcome_Paired) _then;

/// Create a copy of OfferOutcome
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? device = null,}) {
  return _then(OfferOutcome_Paired(
device: null == device ? _self.device : device // ignore: cast_nullable_to_non_nullable
as Device,
  ));
}


}

/// @nodoc


class OfferOutcome_Expired extends OfferOutcome {
  const OfferOutcome_Expired(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is OfferOutcome_Expired);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'OfferOutcome.expired()';
}


}




/// @nodoc


class OfferOutcome_Cancelled extends OfferOutcome {
  const OfferOutcome_Cancelled(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is OfferOutcome_Cancelled);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'OfferOutcome.cancelled()';
}


}




/// @nodoc


class OfferOutcome_Declined extends OfferOutcome {
  const OfferOutcome_Declined(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is OfferOutcome_Declined);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'OfferOutcome.declined()';
}


}




/// @nodoc


class OfferOutcome_Abandoned extends OfferOutcome {
  const OfferOutcome_Abandoned(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is OfferOutcome_Abandoned);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'OfferOutcome.abandoned()';
}


}




/// @nodoc


class OfferOutcome_Failed extends OfferOutcome {
  const OfferOutcome_Failed({required this.message}): super._();
  

/// One sentence that says what.
 final  String message;

/// Create a copy of OfferOutcome
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$OfferOutcome_FailedCopyWith<OfferOutcome_Failed> get copyWith => _$OfferOutcome_FailedCopyWithImpl<OfferOutcome_Failed>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is OfferOutcome_Failed&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode {
    return Object.hash(runtimeType,message);
}

@override
String toString() {
    return 'OfferOutcome.failed(message: $message)';
}


}

/// @nodoc
abstract mixin class $OfferOutcome_FailedCopyWith<$Res> implements $OfferOutcomeCopyWith<$Res> {
  factory $OfferOutcome_FailedCopyWith(OfferOutcome_Failed value, $Res Function(OfferOutcome_Failed) _then) = _$OfferOutcome_FailedCopyWithImpl;
@useResult
$Res call({
 String message
});




}
/// @nodoc
class _$OfferOutcome_FailedCopyWithImpl<$Res>
    implements $OfferOutcome_FailedCopyWith<$Res> {
  _$OfferOutcome_FailedCopyWithImpl(this._self, this._then);

  final OfferOutcome_Failed _self;
  final $Res Function(OfferOutcome_Failed) _then;

/// Create a copy of OfferOutcome
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? message = null,}) {
  return _then(OfferOutcome_Failed(
message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc
mixin _$ReceivedItem {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ReceivedItem);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'ReceivedItem()';
}


}

/// @nodoc
class $ReceivedItemCopyWith<$Res>  {
$ReceivedItemCopyWith(ReceivedItem _, $Res Function(ReceivedItem) __);
}


/// Adds pattern-matching-related methods to [ReceivedItem].
extension ReceivedItemPatterns on ReceivedItem {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( ReceivedItem_Link value)?  link,TResult Function( ReceivedItem_Text value)?  text,required TResult orElse(),}){
final _that = this;
switch (_that) {
case ReceivedItem_Link() when link != null:
return link(_that);case ReceivedItem_Text() when text != null:
return text(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( ReceivedItem_Link value)  link,required TResult Function( ReceivedItem_Text value)  text,}){
final _that = this;
switch (_that) {
case ReceivedItem_Link():
return link(_that);case ReceivedItem_Text():
return text(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( ReceivedItem_Link value)?  link,TResult? Function( ReceivedItem_Text value)?  text,}){
final _that = this;
switch (_that) {
case ReceivedItem_Link() when link != null:
return link(_that);case ReceivedItem_Text() when text != null:
return text(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( String url,  String? title)?  link,TResult Function( String text)?  text,required TResult orElse(),}) {final _that = this;
switch (_that) {
case ReceivedItem_Link() when link != null:
return link(_that.url,_that.title);case ReceivedItem_Text() when text != null:
return text(_that.text);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( String url,  String? title)  link,required TResult Function( String text)  text,}) {final _that = this;
switch (_that) {
case ReceivedItem_Link():
return link(_that.url,_that.title);case ReceivedItem_Text():
return text(_that.text);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( String url,  String? title)?  link,TResult? Function( String text)?  text,}) {final _that = this;
switch (_that) {
case ReceivedItem_Link() when link != null:
return link(_that.url,_that.title);case ReceivedItem_Text() when text != null:
return text(_that.text);case _:
  return null;

}
}

}

/// @nodoc


class ReceivedItem_Link extends ReceivedItem {
  const ReceivedItem_Link({required this.url, this.title}): super._();
  

/// The address, in the plain ASCII form a browser would send.
 final  String url;
/// The page title, if the sender knew one.
 final  String? title;

/// Create a copy of ReceivedItem
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ReceivedItem_LinkCopyWith<ReceivedItem_Link> get copyWith => _$ReceivedItem_LinkCopyWithImpl<ReceivedItem_Link>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ReceivedItem_Link&&(identical(other.url, url) || other.url == url)&&(identical(other.title, title) || other.title == title));
}


@override
int get hashCode {
    return Object.hash(runtimeType,url,title);
}

@override
String toString() {
    return 'ReceivedItem.link(url: $url, title: $title)';
}


}

/// @nodoc
abstract mixin class $ReceivedItem_LinkCopyWith<$Res> implements $ReceivedItemCopyWith<$Res> {
  factory $ReceivedItem_LinkCopyWith(ReceivedItem_Link value, $Res Function(ReceivedItem_Link) _then) = _$ReceivedItem_LinkCopyWithImpl;
@useResult
$Res call({
 String url, String? title
});




}
/// @nodoc
class _$ReceivedItem_LinkCopyWithImpl<$Res>
    implements $ReceivedItem_LinkCopyWith<$Res> {
  _$ReceivedItem_LinkCopyWithImpl(this._self, this._then);

  final ReceivedItem_Link _self;
  final $Res Function(ReceivedItem_Link) _then;

/// Create a copy of ReceivedItem
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? url = null,Object? title = freezed,}) {
  return _then(ReceivedItem_Link(
url: null == url ? _self.url : url // ignore: cast_nullable_to_non_nullable
as String,title: freezed == title ? _self.title : title // ignore: cast_nullable_to_non_nullable
as String?,
  ));
}


}

/// @nodoc


class ReceivedItem_Text extends ReceivedItem {
  const ReceivedItem_Text({required this.text}): super._();
  

/// The text, exactly as sent.
 final  String text;

/// Create a copy of ReceivedItem
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ReceivedItem_TextCopyWith<ReceivedItem_Text> get copyWith => _$ReceivedItem_TextCopyWithImpl<ReceivedItem_Text>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ReceivedItem_Text&&(identical(other.text, text) || other.text == text));
}


@override
int get hashCode {
    return Object.hash(runtimeType,text);
}

@override
String toString() {
    return 'ReceivedItem.text(text: $text)';
}


}

/// @nodoc
abstract mixin class $ReceivedItem_TextCopyWith<$Res> implements $ReceivedItemCopyWith<$Res> {
  factory $ReceivedItem_TextCopyWith(ReceivedItem_Text value, $Res Function(ReceivedItem_Text) _then) = _$ReceivedItem_TextCopyWithImpl;
@useResult
$Res call({
 String text
});




}
/// @nodoc
class _$ReceivedItem_TextCopyWithImpl<$Res>
    implements $ReceivedItem_TextCopyWith<$Res> {
  _$ReceivedItem_TextCopyWithImpl(this._self, this._then);

  final ReceivedItem_Text _self;
  final $Res Function(ReceivedItem_Text) _then;

/// Create a copy of ReceivedItem
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? text = null,}) {
  return _then(ReceivedItem_Text(
text: null == text ? _self.text : text // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc
mixin _$Receiving {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Receiving);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'Receiving()';
}


}

/// @nodoc
class $ReceivingCopyWith<$Res>  {
$ReceivingCopyWith(Receiving _, $Res Function(Receiving) __);
}


/// Adds pattern-matching-related methods to [Receiving].
extension ReceivingPatterns on Receiving {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( Receiving_Off value)?  off,TResult Function( Receiving_On value)?  on_,TResult Function( Receiving_Unusable value)?  unusable,required TResult orElse(),}){
final _that = this;
switch (_that) {
case Receiving_Off() when off != null:
return off(_that);case Receiving_On() when on_ != null:
return on_(_that);case Receiving_Unusable() when unusable != null:
return unusable(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( Receiving_Off value)  off,required TResult Function( Receiving_On value)  on_,required TResult Function( Receiving_Unusable value)  unusable,}){
final _that = this;
switch (_that) {
case Receiving_Off():
return off(_that);case Receiving_On():
return on_(_that);case Receiving_Unusable():
return unusable(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( Receiving_Off value)?  off,TResult? Function( Receiving_On value)?  on_,TResult? Function( Receiving_Unusable value)?  unusable,}){
final _that = this;
switch (_that) {
case Receiving_Off() when off != null:
return off(_that);case Receiving_On() when on_ != null:
return on_(_that);case Receiving_Unusable() when unusable != null:
return unusable(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function()?  off,TResult Function()?  on_,TResult Function( String message)?  unusable,required TResult orElse(),}) {final _that = this;
switch (_that) {
case Receiving_Off() when off != null:
return off();case Receiving_On() when on_ != null:
return on_();case Receiving_Unusable() when unusable != null:
return unusable(_that.message);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function()  off,required TResult Function()  on_,required TResult Function( String message)  unusable,}) {final _that = this;
switch (_that) {
case Receiving_Off():
return off();case Receiving_On():
return on_();case Receiving_Unusable():
return unusable(_that.message);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function()?  off,TResult? Function()?  on_,TResult? Function( String message)?  unusable,}) {final _that = this;
switch (_that) {
case Receiving_Off() when off != null:
return off();case Receiving_On() when on_ != null:
return on_();case Receiving_Unusable() when unusable != null:
return unusable(_that.message);case _:
  return null;

}
}

}

/// @nodoc


class Receiving_Off extends Receiving {
  const Receiving_Off(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Receiving_Off);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'Receiving.off()';
}


}




/// @nodoc


class Receiving_On extends Receiving {
  const Receiving_On(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Receiving_On);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'Receiving.on_()';
}


}




/// @nodoc


class Receiving_Unusable extends Receiving {
  const Receiving_Unusable({required this.message}): super._();
  

/// One sentence that says why.
 final  String message;

/// Create a copy of Receiving
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$Receiving_UnusableCopyWith<Receiving_Unusable> get copyWith => _$Receiving_UnusableCopyWithImpl<Receiving_Unusable>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Receiving_Unusable&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode {
    return Object.hash(runtimeType,message);
}

@override
String toString() {
    return 'Receiving.unusable(message: $message)';
}


}

/// @nodoc
abstract mixin class $Receiving_UnusableCopyWith<$Res> implements $ReceivingCopyWith<$Res> {
  factory $Receiving_UnusableCopyWith(Receiving_Unusable value, $Res Function(Receiving_Unusable) _then) = _$Receiving_UnusableCopyWithImpl;
@useResult
$Res call({
 String message
});




}
/// @nodoc
class _$Receiving_UnusableCopyWithImpl<$Res>
    implements $Receiving_UnusableCopyWith<$Res> {
  _$Receiving_UnusableCopyWithImpl(this._self, this._then);

  final Receiving_Unusable _self;
  final $Res Function(Receiving_Unusable) _then;

/// Create a copy of Receiving
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? message = null,}) {
  return _then(Receiving_Unusable(
message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc
mixin _$ScreenStarted {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ScreenStarted);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'ScreenStarted()';
}


}

/// @nodoc
class $ScreenStartedCopyWith<$Res>  {
$ScreenStartedCopyWith(ScreenStarted _, $Res Function(ScreenStarted) __);
}


/// Adds pattern-matching-related methods to [ScreenStarted].
extension ScreenStartedPatterns on ScreenStarted {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( ScreenStarted_Started value)?  started,TResult Function( ScreenStarted_NotStarted value)?  notStarted,required TResult orElse(),}){
final _that = this;
switch (_that) {
case ScreenStarted_Started() when started != null:
return started(_that);case ScreenStarted_NotStarted() when notStarted != null:
return notStarted(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( ScreenStarted_Started value)  started,required TResult Function( ScreenStarted_NotStarted value)  notStarted,}){
final _that = this;
switch (_that) {
case ScreenStarted_Started():
return started(_that);case ScreenStarted_NotStarted():
return notStarted(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( ScreenStarted_Started value)?  started,TResult? Function( ScreenStarted_NotStarted value)?  notStarted,}){
final _that = this;
switch (_that) {
case ScreenStarted_Started() when started != null:
return started(_that);case ScreenStarted_NotStarted() when notStarted != null:
return notStarted(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( WindowSession session)?  started,TResult Function( String message)?  notStarted,required TResult orElse(),}) {final _that = this;
switch (_that) {
case ScreenStarted_Started() when started != null:
return started(_that.session);case ScreenStarted_NotStarted() when notStarted != null:
return notStarted(_that.message);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( WindowSession session)  started,required TResult Function( String message)  notStarted,}) {final _that = this;
switch (_that) {
case ScreenStarted_Started():
return started(_that.session);case ScreenStarted_NotStarted():
return notStarted(_that.message);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( WindowSession session)?  started,TResult? Function( String message)?  notStarted,}) {final _that = this;
switch (_that) {
case ScreenStarted_Started() when started != null:
return started(_that.session);case ScreenStarted_NotStarted() when notStarted != null:
return notStarted(_that.message);case _:
  return null;

}
}

}

/// @nodoc


class ScreenStarted_Started extends ScreenStarted {
  const ScreenStarted_Started({required this.session}): super._();
  

/// The session.
 final  WindowSession session;

/// Create a copy of ScreenStarted
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ScreenStarted_StartedCopyWith<ScreenStarted_Started> get copyWith => _$ScreenStarted_StartedCopyWithImpl<ScreenStarted_Started>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ScreenStarted_Started&&(identical(other.session, session) || other.session == session));
}


@override
int get hashCode {
    return Object.hash(runtimeType,session);
}

@override
String toString() {
    return 'ScreenStarted.started(session: $session)';
}


}

/// @nodoc
abstract mixin class $ScreenStarted_StartedCopyWith<$Res> implements $ScreenStartedCopyWith<$Res> {
  factory $ScreenStarted_StartedCopyWith(ScreenStarted_Started value, $Res Function(ScreenStarted_Started) _then) = _$ScreenStarted_StartedCopyWithImpl;
@useResult
$Res call({
 WindowSession session
});




}
/// @nodoc
class _$ScreenStarted_StartedCopyWithImpl<$Res>
    implements $ScreenStarted_StartedCopyWith<$Res> {
  _$ScreenStarted_StartedCopyWithImpl(this._self, this._then);

  final ScreenStarted_Started _self;
  final $Res Function(ScreenStarted_Started) _then;

/// Create a copy of ScreenStarted
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? session = null,}) {
  return _then(ScreenStarted_Started(
session: null == session ? _self.session : session // ignore: cast_nullable_to_non_nullable
as WindowSession,
  ));
}


}

/// @nodoc


class ScreenStarted_NotStarted extends ScreenStarted {
  const ScreenStarted_NotStarted({required this.message}): super._();
  

/// One sentence that says why, in words that name the device and tell the person what to
/// do about it.
 final  String message;

/// Create a copy of ScreenStarted
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ScreenStarted_NotStartedCopyWith<ScreenStarted_NotStarted> get copyWith => _$ScreenStarted_NotStartedCopyWithImpl<ScreenStarted_NotStarted>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ScreenStarted_NotStarted&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode {
    return Object.hash(runtimeType,message);
}

@override
String toString() {
    return 'ScreenStarted.notStarted(message: $message)';
}


}

/// @nodoc
abstract mixin class $ScreenStarted_NotStartedCopyWith<$Res> implements $ScreenStartedCopyWith<$Res> {
  factory $ScreenStarted_NotStartedCopyWith(ScreenStarted_NotStarted value, $Res Function(ScreenStarted_NotStarted) _then) = _$ScreenStarted_NotStartedCopyWithImpl;
@useResult
$Res call({
 String message
});




}
/// @nodoc
class _$ScreenStarted_NotStartedCopyWithImpl<$Res>
    implements $ScreenStarted_NotStartedCopyWith<$Res> {
  _$ScreenStarted_NotStartedCopyWithImpl(this._self, this._then);

  final ScreenStarted_NotStarted _self;
  final $Res Function(ScreenStarted_NotStarted) _then;

/// Create a copy of ScreenStarted
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? message = null,}) {
  return _then(ScreenStarted_NotStarted(
message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc
mixin _$TransferResult {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is TransferResult);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'TransferResult()';
}


}

/// @nodoc
class $TransferResultCopyWith<$Res>  {
$TransferResultCopyWith(TransferResult _, $Res Function(TransferResult) __);
}


/// Adds pattern-matching-related methods to [TransferResult].
extension TransferResultPatterns on TransferResult {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( TransferResult_Received value)?  received,TResult Function( TransferResult_Dropped value)?  dropped,required TResult orElse(),}){
final _that = this;
switch (_that) {
case TransferResult_Received() when received != null:
return received(_that);case TransferResult_Dropped() when dropped != null:
return dropped(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( TransferResult_Received value)  received,required TResult Function( TransferResult_Dropped value)  dropped,}){
final _that = this;
switch (_that) {
case TransferResult_Received():
return received(_that);case TransferResult_Dropped():
return dropped(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( TransferResult_Received value)?  received,TResult? Function( TransferResult_Dropped value)?  dropped,}){
final _that = this;
switch (_that) {
case TransferResult_Received() when received != null:
return received(_that);case TransferResult_Dropped() when dropped != null:
return dropped(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( int failed)?  received,TResult Function()?  dropped,required TResult orElse(),}) {final _that = this;
switch (_that) {
case TransferResult_Received() when received != null:
return received(_that.failed);case TransferResult_Dropped() when dropped != null:
return dropped();case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( int failed)  received,required TResult Function()  dropped,}) {final _that = this;
switch (_that) {
case TransferResult_Received():
return received(_that.failed);case TransferResult_Dropped():
return dropped();}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( int failed)?  received,TResult? Function()?  dropped,}) {final _that = this;
switch (_that) {
case TransferResult_Received() when received != null:
return received(_that.failed);case TransferResult_Dropped() when dropped != null:
return dropped();case _:
  return null;

}
}

}

/// @nodoc


class TransferResult_Received extends TransferResult {
  const TransferResult_Received({required this.failed}): super._();
  

/// How many files did not arrive.
 final  int failed;

/// Create a copy of TransferResult
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$TransferResult_ReceivedCopyWith<TransferResult_Received> get copyWith => _$TransferResult_ReceivedCopyWithImpl<TransferResult_Received>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is TransferResult_Received&&(identical(other.failed, failed) || other.failed == failed));
}


@override
int get hashCode {
    return Object.hash(runtimeType,failed);
}

@override
String toString() {
    return 'TransferResult.received(failed: $failed)';
}


}

/// @nodoc
abstract mixin class $TransferResult_ReceivedCopyWith<$Res> implements $TransferResultCopyWith<$Res> {
  factory $TransferResult_ReceivedCopyWith(TransferResult_Received value, $Res Function(TransferResult_Received) _then) = _$TransferResult_ReceivedCopyWithImpl;
@useResult
$Res call({
 int failed
});




}
/// @nodoc
class _$TransferResult_ReceivedCopyWithImpl<$Res>
    implements $TransferResult_ReceivedCopyWith<$Res> {
  _$TransferResult_ReceivedCopyWithImpl(this._self, this._then);

  final TransferResult_Received _self;
  final $Res Function(TransferResult_Received) _then;

/// Create a copy of TransferResult
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? failed = null,}) {
  return _then(TransferResult_Received(
failed: null == failed ? _self.failed : failed // ignore: cast_nullable_to_non_nullable
as int,
  ));
}


}

/// @nodoc


class TransferResult_Dropped extends TransferResult {
  const TransferResult_Dropped(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is TransferResult_Dropped);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'TransferResult.dropped()';
}


}




/// @nodoc
mixin _$UnlockSetUp {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is UnlockSetUp);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'UnlockSetUp()';
}


}

/// @nodoc
class $UnlockSetUpCopyWith<$Res>  {
$UnlockSetUpCopyWith(UnlockSetUp _, $Res Function(UnlockSetUp) __);
}


/// Adds pattern-matching-related methods to [UnlockSetUp].
extension UnlockSetUpPatterns on UnlockSetUp {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( UnlockSetUp_Done value)?  done,TResult Function( UnlockSetUp_WrongPassword value)?  wrongPassword,TResult Function( UnlockSetUp_NotDone value)?  notDone,required TResult orElse(),}){
final _that = this;
switch (_that) {
case UnlockSetUp_Done() when done != null:
return done(_that);case UnlockSetUp_WrongPassword() when wrongPassword != null:
return wrongPassword(_that);case UnlockSetUp_NotDone() when notDone != null:
return notDone(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( UnlockSetUp_Done value)  done,required TResult Function( UnlockSetUp_WrongPassword value)  wrongPassword,required TResult Function( UnlockSetUp_NotDone value)  notDone,}){
final _that = this;
switch (_that) {
case UnlockSetUp_Done():
return done(_that);case UnlockSetUp_WrongPassword():
return wrongPassword(_that);case UnlockSetUp_NotDone():
return notDone(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( UnlockSetUp_Done value)?  done,TResult? Function( UnlockSetUp_WrongPassword value)?  wrongPassword,TResult? Function( UnlockSetUp_NotDone value)?  notDone,}){
final _that = this;
switch (_that) {
case UnlockSetUp_Done() when done != null:
return done(_that);case UnlockSetUp_WrongPassword() when wrongPassword != null:
return wrongPassword(_that);case UnlockSetUp_NotDone() when notDone != null:
return notDone(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function()?  done,TResult Function()?  wrongPassword,TResult Function( String message)?  notDone,required TResult orElse(),}) {final _that = this;
switch (_that) {
case UnlockSetUp_Done() when done != null:
return done();case UnlockSetUp_WrongPassword() when wrongPassword != null:
return wrongPassword();case UnlockSetUp_NotDone() when notDone != null:
return notDone(_that.message);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function()  done,required TResult Function()  wrongPassword,required TResult Function( String message)  notDone,}) {final _that = this;
switch (_that) {
case UnlockSetUp_Done():
return done();case UnlockSetUp_WrongPassword():
return wrongPassword();case UnlockSetUp_NotDone():
return notDone(_that.message);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function()?  done,TResult? Function()?  wrongPassword,TResult? Function( String message)?  notDone,}) {final _that = this;
switch (_that) {
case UnlockSetUp_Done() when done != null:
return done();case UnlockSetUp_WrongPassword() when wrongPassword != null:
return wrongPassword();case UnlockSetUp_NotDone() when notDone != null:
return notDone(_that.message);case _:
  return null;

}
}

}

/// @nodoc


class UnlockSetUp_Done extends UnlockSetUp {
  const UnlockSetUp_Done(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is UnlockSetUp_Done);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'UnlockSetUp.done()';
}


}




/// @nodoc


class UnlockSetUp_WrongPassword extends UnlockSetUp {
  const UnlockSetUp_WrongPassword(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is UnlockSetUp_WrongPassword);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'UnlockSetUp.wrongPassword()';
}


}




/// @nodoc


class UnlockSetUp_NotDone extends UnlockSetUp {
  const UnlockSetUp_NotDone({required this.message}): super._();
  

/// One sentence that says why, in words that name the device.
 final  String message;

/// Create a copy of UnlockSetUp
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$UnlockSetUp_NotDoneCopyWith<UnlockSetUp_NotDone> get copyWith => _$UnlockSetUp_NotDoneCopyWithImpl<UnlockSetUp_NotDone>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is UnlockSetUp_NotDone&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode {
    return Object.hash(runtimeType,message);
}

@override
String toString() {
    return 'UnlockSetUp.notDone(message: $message)';
}


}

/// @nodoc
abstract mixin class $UnlockSetUp_NotDoneCopyWith<$Res> implements $UnlockSetUpCopyWith<$Res> {
  factory $UnlockSetUp_NotDoneCopyWith(UnlockSetUp_NotDone value, $Res Function(UnlockSetUp_NotDone) _then) = _$UnlockSetUp_NotDoneCopyWithImpl;
@useResult
$Res call({
 String message
});




}
/// @nodoc
class _$UnlockSetUp_NotDoneCopyWithImpl<$Res>
    implements $UnlockSetUp_NotDoneCopyWith<$Res> {
  _$UnlockSetUp_NotDoneCopyWithImpl(this._self, this._then);

  final UnlockSetUp_NotDone _self;
  final $Res Function(UnlockSetUp_NotDone) _then;

/// Create a copy of UnlockSetUp
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? message = null,}) {
  return _then(UnlockSetUp_NotDone(
message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc
mixin _$UnlockStatus {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is UnlockStatus);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'UnlockStatus()';
}


}

/// @nodoc
class $UnlockStatusCopyWith<$Res>  {
$UnlockStatusCopyWith(UnlockStatus _, $Res Function(UnlockStatus) __);
}


/// Adds pattern-matching-related methods to [UnlockStatus].
extension UnlockStatusPatterns on UnlockStatus {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( UnlockStatus_Off value)?  off,TResult Function( UnlockStatus_On value)?  on_,TResult Function( UnlockStatus_Ended value)?  ended,required TResult orElse(),}){
final _that = this;
switch (_that) {
case UnlockStatus_Off() when off != null:
return off(_that);case UnlockStatus_On() when on_ != null:
return on_(_that);case UnlockStatus_Ended() when ended != null:
return ended(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( UnlockStatus_Off value)  off,required TResult Function( UnlockStatus_On value)  on_,required TResult Function( UnlockStatus_Ended value)  ended,}){
final _that = this;
switch (_that) {
case UnlockStatus_Off():
return off(_that);case UnlockStatus_On():
return on_(_that);case UnlockStatus_Ended():
return ended(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( UnlockStatus_Off value)?  off,TResult? Function( UnlockStatus_On value)?  on_,TResult? Function( UnlockStatus_Ended value)?  ended,}){
final _that = this;
switch (_that) {
case UnlockStatus_Off() when off != null:
return off(_that);case UnlockStatus_On() when on_ != null:
return on_(_that);case UnlockStatus_Ended() when ended != null:
return ended(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function()?  off,TResult Function( PlatformInt64 since)?  on_,TResult Function( PlatformInt64 since)?  ended,required TResult orElse(),}) {final _that = this;
switch (_that) {
case UnlockStatus_Off() when off != null:
return off();case UnlockStatus_On() when on_ != null:
return on_(_that.since);case UnlockStatus_Ended() when ended != null:
return ended(_that.since);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function()  off,required TResult Function( PlatformInt64 since)  on_,required TResult Function( PlatformInt64 since)  ended,}) {final _that = this;
switch (_that) {
case UnlockStatus_Off():
return off();case UnlockStatus_On():
return on_(_that.since);case UnlockStatus_Ended():
return ended(_that.since);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function()?  off,TResult? Function( PlatformInt64 since)?  on_,TResult? Function( PlatformInt64 since)?  ended,}) {final _that = this;
switch (_that) {
case UnlockStatus_Off() when off != null:
return off();case UnlockStatus_On() when on_ != null:
return on_(_that.since);case UnlockStatus_Ended() when ended != null:
return ended(_that.since);case _:
  return null;

}
}

}

/// @nodoc


class UnlockStatus_Off extends UnlockStatus {
  const UnlockStatus_Off(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is UnlockStatus_Off);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'UnlockStatus.off()';
}


}




/// @nodoc


class UnlockStatus_On extends UnlockStatus {
  const UnlockStatus_On({required this.since}): super._();
  

/// When the person set it up, in seconds since the Unix epoch.
 final  PlatformInt64 since;

/// Create a copy of UnlockStatus
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$UnlockStatus_OnCopyWith<UnlockStatus_On> get copyWith => _$UnlockStatus_OnCopyWithImpl<UnlockStatus_On>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is UnlockStatus_On&&(identical(other.since, since) || other.since == since));
}


@override
int get hashCode {
    return Object.hash(runtimeType,since);
}

@override
String toString() {
    return 'UnlockStatus.on_(since: $since)';
}


}

/// @nodoc
abstract mixin class $UnlockStatus_OnCopyWith<$Res> implements $UnlockStatusCopyWith<$Res> {
  factory $UnlockStatus_OnCopyWith(UnlockStatus_On value, $Res Function(UnlockStatus_On) _then) = _$UnlockStatus_OnCopyWithImpl;
@useResult
$Res call({
 PlatformInt64 since
});




}
/// @nodoc
class _$UnlockStatus_OnCopyWithImpl<$Res>
    implements $UnlockStatus_OnCopyWith<$Res> {
  _$UnlockStatus_OnCopyWithImpl(this._self, this._then);

  final UnlockStatus_On _self;
  final $Res Function(UnlockStatus_On) _then;

/// Create a copy of UnlockStatus
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? since = null,}) {
  return _then(UnlockStatus_On(
since: null == since ? _self.since : since // ignore: cast_nullable_to_non_nullable
as PlatformInt64,
  ));
}


}

/// @nodoc


class UnlockStatus_Ended extends UnlockStatus {
  const UnlockStatus_Ended({required this.since}): super._();
  

/// When the person set it up, in seconds since the Unix epoch.
 final  PlatformInt64 since;

/// Create a copy of UnlockStatus
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$UnlockStatus_EndedCopyWith<UnlockStatus_Ended> get copyWith => _$UnlockStatus_EndedCopyWithImpl<UnlockStatus_Ended>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is UnlockStatus_Ended&&(identical(other.since, since) || other.since == since));
}


@override
int get hashCode {
    return Object.hash(runtimeType,since);
}

@override
String toString() {
    return 'UnlockStatus.ended(since: $since)';
}


}

/// @nodoc
abstract mixin class $UnlockStatus_EndedCopyWith<$Res> implements $UnlockStatusCopyWith<$Res> {
  factory $UnlockStatus_EndedCopyWith(UnlockStatus_Ended value, $Res Function(UnlockStatus_Ended) _then) = _$UnlockStatus_EndedCopyWithImpl;
@useResult
$Res call({
 PlatformInt64 since
});




}
/// @nodoc
class _$UnlockStatus_EndedCopyWithImpl<$Res>
    implements $UnlockStatus_EndedCopyWith<$Res> {
  _$UnlockStatus_EndedCopyWithImpl(this._self, this._then);

  final UnlockStatus_Ended _self;
  final $Res Function(UnlockStatus_Ended) _then;

/// Create a copy of UnlockStatus
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? since = null,}) {
  return _then(UnlockStatus_Ended(
since: null == since ? _self.since : since // ignore: cast_nullable_to_non_nullable
as PlatformInt64,
  ));
}


}

/// @nodoc
mixin _$WindowEnd {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is WindowEnd);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'WindowEnd()';
}


}

/// @nodoc
class $WindowEndCopyWith<$Res>  {
$WindowEndCopyWith(WindowEnd _, $Res Function(WindowEnd) __);
}


/// Adds pattern-matching-related methods to [WindowEnd].
extension WindowEndPatterns on WindowEnd {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( WindowEnd_ThisDevice value)?  thisDevice,TResult Function( WindowEnd_OtherDevice value)?  otherDevice,TResult Function( WindowEnd_Lost value)?  lost,TResult Function( WindowEnd_Failed value)?  failed,required TResult orElse(),}){
final _that = this;
switch (_that) {
case WindowEnd_ThisDevice() when thisDevice != null:
return thisDevice(_that);case WindowEnd_OtherDevice() when otherDevice != null:
return otherDevice(_that);case WindowEnd_Lost() when lost != null:
return lost(_that);case WindowEnd_Failed() when failed != null:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( WindowEnd_ThisDevice value)  thisDevice,required TResult Function( WindowEnd_OtherDevice value)  otherDevice,required TResult Function( WindowEnd_Lost value)  lost,required TResult Function( WindowEnd_Failed value)  failed,}){
final _that = this;
switch (_that) {
case WindowEnd_ThisDevice():
return thisDevice(_that);case WindowEnd_OtherDevice():
return otherDevice(_that);case WindowEnd_Lost():
return lost(_that);case WindowEnd_Failed():
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( WindowEnd_ThisDevice value)?  thisDevice,TResult? Function( WindowEnd_OtherDevice value)?  otherDevice,TResult? Function( WindowEnd_Lost value)?  lost,TResult? Function( WindowEnd_Failed value)?  failed,}){
final _that = this;
switch (_that) {
case WindowEnd_ThisDevice() when thisDevice != null:
return thisDevice(_that);case WindowEnd_OtherDevice() when otherDevice != null:
return otherDevice(_that);case WindowEnd_Lost() when lost != null:
return lost(_that);case WindowEnd_Failed() when failed != null:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function()?  thisDevice,TResult Function()?  otherDevice,TResult Function()?  lost,TResult Function( String message)?  failed,required TResult orElse(),}) {final _that = this;
switch (_that) {
case WindowEnd_ThisDevice() when thisDevice != null:
return thisDevice();case WindowEnd_OtherDevice() when otherDevice != null:
return otherDevice();case WindowEnd_Lost() when lost != null:
return lost();case WindowEnd_Failed() when failed != null:
return failed(_that.message);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function()  thisDevice,required TResult Function()  otherDevice,required TResult Function()  lost,required TResult Function( String message)  failed,}) {final _that = this;
switch (_that) {
case WindowEnd_ThisDevice():
return thisDevice();case WindowEnd_OtherDevice():
return otherDevice();case WindowEnd_Lost():
return lost();case WindowEnd_Failed():
return failed(_that.message);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function()?  thisDevice,TResult? Function()?  otherDevice,TResult? Function()?  lost,TResult? Function( String message)?  failed,}) {final _that = this;
switch (_that) {
case WindowEnd_ThisDevice() when thisDevice != null:
return thisDevice();case WindowEnd_OtherDevice() when otherDevice != null:
return otherDevice();case WindowEnd_Lost() when lost != null:
return lost();case WindowEnd_Failed() when failed != null:
return failed(_that.message);case _:
  return null;

}
}

}

/// @nodoc


class WindowEnd_ThisDevice extends WindowEnd {
  const WindowEnd_ThisDevice(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is WindowEnd_ThisDevice);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'WindowEnd.thisDevice()';
}


}




/// @nodoc


class WindowEnd_OtherDevice extends WindowEnd {
  const WindowEnd_OtherDevice(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is WindowEnd_OtherDevice);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'WindowEnd.otherDevice()';
}


}




/// @nodoc


class WindowEnd_Lost extends WindowEnd {
  const WindowEnd_Lost(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is WindowEnd_Lost);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'WindowEnd.lost()';
}


}




/// @nodoc


class WindowEnd_Failed extends WindowEnd {
  const WindowEnd_Failed({required this.message}): super._();
  

/// One sentence that says what.
 final  String message;

/// Create a copy of WindowEnd
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$WindowEnd_FailedCopyWith<WindowEnd_Failed> get copyWith => _$WindowEnd_FailedCopyWithImpl<WindowEnd_Failed>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is WindowEnd_Failed&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode {
    return Object.hash(runtimeType,message);
}

@override
String toString() {
    return 'WindowEnd.failed(message: $message)';
}


}

/// @nodoc
abstract mixin class $WindowEnd_FailedCopyWith<$Res> implements $WindowEndCopyWith<$Res> {
  factory $WindowEnd_FailedCopyWith(WindowEnd_Failed value, $Res Function(WindowEnd_Failed) _then) = _$WindowEnd_FailedCopyWithImpl;
@useResult
$Res call({
 String message
});




}
/// @nodoc
class _$WindowEnd_FailedCopyWithImpl<$Res>
    implements $WindowEnd_FailedCopyWith<$Res> {
  _$WindowEnd_FailedCopyWithImpl(this._self, this._then);

  final WindowEnd_Failed _self;
  final $Res Function(WindowEnd_Failed) _then;

/// Create a copy of WindowEnd
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? message = null,}) {
  return _then(WindowEnd_Failed(
message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc
mixin _$WindowInput {

/// The modifier keys that are down.
 WindowModifiers get modifiers;
/// Create a copy of WindowInput
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$WindowInputCopyWith<WindowInput> get copyWith => _$WindowInputCopyWithImpl<WindowInput>(this as WindowInput, _$identity);



@override
bool operator ==(Object other) {
  final _this = this as WindowInput;
  return identical(this, other) || (other.runtimeType == runtimeType&&other is WindowInput&&(identical(other.modifiers, _this.modifiers) || other.modifiers == _this.modifiers));
}


@override
int get hashCode {
  final _this = this as WindowInput;
  return Object.hash(runtimeType,_this.modifiers);
}

@override
String toString() {
  final _this = this as WindowInput;
  return 'WindowInput(modifiers: ${_this.modifiers})';
}


}

/// @nodoc
abstract mixin class $WindowInputCopyWith<$Res>  {
  factory $WindowInputCopyWith(WindowInput value, $Res Function(WindowInput) _then) = _$WindowInputCopyWithImpl;
@useResult
$Res call({
 WindowModifiers modifiers
});




}
/// @nodoc
class _$WindowInputCopyWithImpl<$Res>
    implements $WindowInputCopyWith<$Res> {
  _$WindowInputCopyWithImpl(this._self, this._then);

  final WindowInput _self;
  final $Res Function(WindowInput) _then;

/// Create a copy of WindowInput
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') @override $Res call({Object? modifiers = null,}) {
  return _then(_self.copyWith(
modifiers: null == modifiers ? _self.modifiers : modifiers // ignore: cast_nullable_to_non_nullable
as WindowModifiers,
  ));
}

}


/// Adds pattern-matching-related methods to [WindowInput].
extension WindowInputPatterns on WindowInput {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( WindowInput_Pointer value)?  pointer,TResult Function( WindowInput_Key value)?  key,required TResult orElse(),}){
final _that = this;
switch (_that) {
case WindowInput_Pointer() when pointer != null:
return pointer(_that);case WindowInput_Key() when key != null:
return key(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( WindowInput_Pointer value)  pointer,required TResult Function( WindowInput_Key value)  key,}){
final _that = this;
switch (_that) {
case WindowInput_Pointer():
return pointer(_that);case WindowInput_Key():
return key(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( WindowInput_Pointer value)?  pointer,TResult? Function( WindowInput_Key value)?  key,}){
final _that = this;
switch (_that) {
case WindowInput_Pointer() when pointer != null:
return pointer(_that);case WindowInput_Key() when key != null:
return key(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( double x,  double y,  WindowModifiers modifiers,  WindowPointer action)?  pointer,TResult Function( int code,  bool down,  WindowModifiers modifiers,  bool repeat,  String? text)?  key,required TResult orElse(),}) {final _that = this;
switch (_that) {
case WindowInput_Pointer() when pointer != null:
return pointer(_that.x,_that.y,_that.modifiers,_that.action);case WindowInput_Key() when key != null:
return key(_that.code,_that.down,_that.modifiers,_that.repeat,_that.text);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( double x,  double y,  WindowModifiers modifiers,  WindowPointer action)  pointer,required TResult Function( int code,  bool down,  WindowModifiers modifiers,  bool repeat,  String? text)  key,}) {final _that = this;
switch (_that) {
case WindowInput_Pointer():
return pointer(_that.x,_that.y,_that.modifiers,_that.action);case WindowInput_Key():
return key(_that.code,_that.down,_that.modifiers,_that.repeat,_that.text);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( double x,  double y,  WindowModifiers modifiers,  WindowPointer action)?  pointer,TResult? Function( int code,  bool down,  WindowModifiers modifiers,  bool repeat,  String? text)?  key,}) {final _that = this;
switch (_that) {
case WindowInput_Pointer() when pointer != null:
return pointer(_that.x,_that.y,_that.modifiers,_that.action);case WindowInput_Key() when key != null:
return key(_that.code,_that.down,_that.modifiers,_that.repeat,_that.text);case _:
  return null;

}
}

}

/// @nodoc


class WindowInput_Pointer extends WindowInput {
  const WindowInput_Pointer({required this.x, required this.y, required this.modifiers, required this.action}): super._();
  

/// Across the picture, from 0 to 1.
 final  double x;
/// Down the picture, from 0 to 1.
 final  double y;
/// The modifier keys that are down.
@override final  WindowModifiers modifiers;
/// What it did.
 final  WindowPointer action;

/// Create a copy of WindowInput
/// with the given fields replaced by the non-null parameter values.
@override @JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$WindowInput_PointerCopyWith<WindowInput_Pointer> get copyWith => _$WindowInput_PointerCopyWithImpl<WindowInput_Pointer>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is WindowInput_Pointer&&(identical(other.x, x) || other.x == x)&&(identical(other.y, y) || other.y == y)&&(identical(other.modifiers, modifiers) || other.modifiers == modifiers)&&(identical(other.action, action) || other.action == action));
}


@override
int get hashCode {
    return Object.hash(runtimeType,x,y,modifiers,action);
}

@override
String toString() {
    return 'WindowInput.pointer(x: $x, y: $y, modifiers: $modifiers, action: $action)';
}


}

/// @nodoc
abstract mixin class $WindowInput_PointerCopyWith<$Res> implements $WindowInputCopyWith<$Res> {
  factory $WindowInput_PointerCopyWith(WindowInput_Pointer value, $Res Function(WindowInput_Pointer) _then) = _$WindowInput_PointerCopyWithImpl;
@override @useResult
$Res call({
 double x, double y, WindowModifiers modifiers, WindowPointer action
});


$WindowPointerCopyWith<$Res> get action;

}
/// @nodoc
class _$WindowInput_PointerCopyWithImpl<$Res>
    implements $WindowInput_PointerCopyWith<$Res> {
  _$WindowInput_PointerCopyWithImpl(this._self, this._then);

  final WindowInput_Pointer _self;
  final $Res Function(WindowInput_Pointer) _then;

/// Create a copy of WindowInput
/// with the given fields replaced by the non-null parameter values.
@override @pragma('vm:prefer-inline') $Res call({Object? x = null,Object? y = null,Object? modifiers = null,Object? action = null,}) {
  return _then(WindowInput_Pointer(
x: null == x ? _self.x : x // ignore: cast_nullable_to_non_nullable
as double,y: null == y ? _self.y : y // ignore: cast_nullable_to_non_nullable
as double,modifiers: null == modifiers ? _self.modifiers : modifiers // ignore: cast_nullable_to_non_nullable
as WindowModifiers,action: null == action ? _self.action : action // ignore: cast_nullable_to_non_nullable
as WindowPointer,
  ));
}

/// Create a copy of WindowInput
/// with the given fields replaced by the non-null parameter values.
@override
@pragma('vm:prefer-inline')
$WindowPointerCopyWith<$Res> get action {
  
  return $WindowPointerCopyWith<$Res>(_self.action, (value) {
    return _then(_self.copyWith(action: value));
  });
}
}

/// @nodoc


class WindowInput_Key extends WindowInput {
  const WindowInput_Key({required this.code, required this.down, required this.modifiers, required this.repeat, this.text}): super._();
  

/// A USB HID usage of the keyboard page, or 0 for text with no key.
 final  int code;
/// Whether the key went down (true) or came up (false).
 final  bool down;
/// The modifier keys that are down after this event.
@override final  WindowModifiers modifiers;
/// Whether the system repeats a key that is held.
 final  bool repeat;
/// The text that the key produced, or `null`. Long text is cut into pieces, and
/// characters that cannot be typed are left out.
 final  String? text;

/// Create a copy of WindowInput
/// with the given fields replaced by the non-null parameter values.
@override @JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$WindowInput_KeyCopyWith<WindowInput_Key> get copyWith => _$WindowInput_KeyCopyWithImpl<WindowInput_Key>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is WindowInput_Key&&(identical(other.code, code) || other.code == code)&&(identical(other.down, down) || other.down == down)&&(identical(other.modifiers, modifiers) || other.modifiers == modifiers)&&(identical(other.repeat, repeat) || other.repeat == repeat)&&(identical(other.text, text) || other.text == text));
}


@override
int get hashCode {
    return Object.hash(runtimeType,code,down,modifiers,repeat,text);
}

@override
String toString() {
    return 'WindowInput.key(code: $code, down: $down, modifiers: $modifiers, repeat: $repeat, text: $text)';
}


}

/// @nodoc
abstract mixin class $WindowInput_KeyCopyWith<$Res> implements $WindowInputCopyWith<$Res> {
  factory $WindowInput_KeyCopyWith(WindowInput_Key value, $Res Function(WindowInput_Key) _then) = _$WindowInput_KeyCopyWithImpl;
@override @useResult
$Res call({
 int code, bool down, WindowModifiers modifiers, bool repeat, String? text
});




}
/// @nodoc
class _$WindowInput_KeyCopyWithImpl<$Res>
    implements $WindowInput_KeyCopyWith<$Res> {
  _$WindowInput_KeyCopyWithImpl(this._self, this._then);

  final WindowInput_Key _self;
  final $Res Function(WindowInput_Key) _then;

/// Create a copy of WindowInput
/// with the given fields replaced by the non-null parameter values.
@override @pragma('vm:prefer-inline') $Res call({Object? code = null,Object? down = null,Object? modifiers = null,Object? repeat = null,Object? text = freezed,}) {
  return _then(WindowInput_Key(
code: null == code ? _self.code : code // ignore: cast_nullable_to_non_nullable
as int,down: null == down ? _self.down : down // ignore: cast_nullable_to_non_nullable
as bool,modifiers: null == modifiers ? _self.modifiers : modifiers // ignore: cast_nullable_to_non_nullable
as WindowModifiers,repeat: null == repeat ? _self.repeat : repeat // ignore: cast_nullable_to_non_nullable
as bool,text: freezed == text ? _self.text : text // ignore: cast_nullable_to_non_nullable
as String?,
  ));
}


}

/// @nodoc
mixin _$WindowPointer {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is WindowPointer);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'WindowPointer()';
}


}

/// @nodoc
class $WindowPointerCopyWith<$Res>  {
$WindowPointerCopyWith(WindowPointer _, $Res Function(WindowPointer) __);
}


/// Adds pattern-matching-related methods to [WindowPointer].
extension WindowPointerPatterns on WindowPointer {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( WindowPointer_Move value)?  move,TResult Function( WindowPointer_Down value)?  down,TResult Function( WindowPointer_Up value)?  up,TResult Function( WindowPointer_Scroll value)?  scroll,required TResult orElse(),}){
final _that = this;
switch (_that) {
case WindowPointer_Move() when move != null:
return move(_that);case WindowPointer_Down() when down != null:
return down(_that);case WindowPointer_Up() when up != null:
return up(_that);case WindowPointer_Scroll() when scroll != null:
return scroll(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( WindowPointer_Move value)  move,required TResult Function( WindowPointer_Down value)  down,required TResult Function( WindowPointer_Up value)  up,required TResult Function( WindowPointer_Scroll value)  scroll,}){
final _that = this;
switch (_that) {
case WindowPointer_Move():
return move(_that);case WindowPointer_Down():
return down(_that);case WindowPointer_Up():
return up(_that);case WindowPointer_Scroll():
return scroll(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( WindowPointer_Move value)?  move,TResult? Function( WindowPointer_Down value)?  down,TResult? Function( WindowPointer_Up value)?  up,TResult? Function( WindowPointer_Scroll value)?  scroll,}){
final _that = this;
switch (_that) {
case WindowPointer_Move() when move != null:
return move(_that);case WindowPointer_Down() when down != null:
return down(_that);case WindowPointer_Up() when up != null:
return up(_that);case WindowPointer_Scroll() when scroll != null:
return scroll(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function()?  move,TResult Function( WindowButton button)?  down,TResult Function( WindowButton button)?  up,TResult Function( double dx,  double dy)?  scroll,required TResult orElse(),}) {final _that = this;
switch (_that) {
case WindowPointer_Move() when move != null:
return move();case WindowPointer_Down() when down != null:
return down(_that.button);case WindowPointer_Up() when up != null:
return up(_that.button);case WindowPointer_Scroll() when scroll != null:
return scroll(_that.dx,_that.dy);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function()  move,required TResult Function( WindowButton button)  down,required TResult Function( WindowButton button)  up,required TResult Function( double dx,  double dy)  scroll,}) {final _that = this;
switch (_that) {
case WindowPointer_Move():
return move();case WindowPointer_Down():
return down(_that.button);case WindowPointer_Up():
return up(_that.button);case WindowPointer_Scroll():
return scroll(_that.dx,_that.dy);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function()?  move,TResult? Function( WindowButton button)?  down,TResult? Function( WindowButton button)?  up,TResult? Function( double dx,  double dy)?  scroll,}) {final _that = this;
switch (_that) {
case WindowPointer_Move() when move != null:
return move();case WindowPointer_Down() when down != null:
return down(_that.button);case WindowPointer_Up() when up != null:
return up(_that.button);case WindowPointer_Scroll() when scroll != null:
return scroll(_that.dx,_that.dy);case _:
  return null;

}
}

}

/// @nodoc


class WindowPointer_Move extends WindowPointer {
  const WindowPointer_Move(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is WindowPointer_Move);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'WindowPointer.move()';
}


}




/// @nodoc


class WindowPointer_Down extends WindowPointer {
  const WindowPointer_Down({required this.button}): super._();
  

/// Which.
 final  WindowButton button;

/// Create a copy of WindowPointer
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$WindowPointer_DownCopyWith<WindowPointer_Down> get copyWith => _$WindowPointer_DownCopyWithImpl<WindowPointer_Down>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is WindowPointer_Down&&(identical(other.button, button) || other.button == button));
}


@override
int get hashCode {
    return Object.hash(runtimeType,button);
}

@override
String toString() {
    return 'WindowPointer.down(button: $button)';
}


}

/// @nodoc
abstract mixin class $WindowPointer_DownCopyWith<$Res> implements $WindowPointerCopyWith<$Res> {
  factory $WindowPointer_DownCopyWith(WindowPointer_Down value, $Res Function(WindowPointer_Down) _then) = _$WindowPointer_DownCopyWithImpl;
@useResult
$Res call({
 WindowButton button
});




}
/// @nodoc
class _$WindowPointer_DownCopyWithImpl<$Res>
    implements $WindowPointer_DownCopyWith<$Res> {
  _$WindowPointer_DownCopyWithImpl(this._self, this._then);

  final WindowPointer_Down _self;
  final $Res Function(WindowPointer_Down) _then;

/// Create a copy of WindowPointer
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? button = null,}) {
  return _then(WindowPointer_Down(
button: null == button ? _self.button : button // ignore: cast_nullable_to_non_nullable
as WindowButton,
  ));
}


}

/// @nodoc


class WindowPointer_Up extends WindowPointer {
  const WindowPointer_Up({required this.button}): super._();
  

/// Which.
 final  WindowButton button;

/// Create a copy of WindowPointer
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$WindowPointer_UpCopyWith<WindowPointer_Up> get copyWith => _$WindowPointer_UpCopyWithImpl<WindowPointer_Up>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is WindowPointer_Up&&(identical(other.button, button) || other.button == button));
}


@override
int get hashCode {
    return Object.hash(runtimeType,button);
}

@override
String toString() {
    return 'WindowPointer.up(button: $button)';
}


}

/// @nodoc
abstract mixin class $WindowPointer_UpCopyWith<$Res> implements $WindowPointerCopyWith<$Res> {
  factory $WindowPointer_UpCopyWith(WindowPointer_Up value, $Res Function(WindowPointer_Up) _then) = _$WindowPointer_UpCopyWithImpl;
@useResult
$Res call({
 WindowButton button
});




}
/// @nodoc
class _$WindowPointer_UpCopyWithImpl<$Res>
    implements $WindowPointer_UpCopyWith<$Res> {
  _$WindowPointer_UpCopyWithImpl(this._self, this._then);

  final WindowPointer_Up _self;
  final $Res Function(WindowPointer_Up) _then;

/// Create a copy of WindowPointer
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? button = null,}) {
  return _then(WindowPointer_Up(
button: null == button ? _self.button : button // ignore: cast_nullable_to_non_nullable
as WindowButton,
  ));
}


}

/// @nodoc


class WindowPointer_Scroll extends WindowPointer {
  const WindowPointer_Scroll({required this.dx, required this.dy}): super._();
  

/// Along the x axis.
 final  double dx;
/// Along the y axis.
 final  double dy;

/// Create a copy of WindowPointer
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$WindowPointer_ScrollCopyWith<WindowPointer_Scroll> get copyWith => _$WindowPointer_ScrollCopyWithImpl<WindowPointer_Scroll>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is WindowPointer_Scroll&&(identical(other.dx, dx) || other.dx == dx)&&(identical(other.dy, dy) || other.dy == dy));
}


@override
int get hashCode {
    return Object.hash(runtimeType,dx,dy);
}

@override
String toString() {
    return 'WindowPointer.scroll(dx: $dx, dy: $dy)';
}


}

/// @nodoc
abstract mixin class $WindowPointer_ScrollCopyWith<$Res> implements $WindowPointerCopyWith<$Res> {
  factory $WindowPointer_ScrollCopyWith(WindowPointer_Scroll value, $Res Function(WindowPointer_Scroll) _then) = _$WindowPointer_ScrollCopyWithImpl;
@useResult
$Res call({
 double dx, double dy
});




}
/// @nodoc
class _$WindowPointer_ScrollCopyWithImpl<$Res>
    implements $WindowPointer_ScrollCopyWith<$Res> {
  _$WindowPointer_ScrollCopyWithImpl(this._self, this._then);

  final WindowPointer_Scroll _self;
  final $Res Function(WindowPointer_Scroll) _then;

/// Create a copy of WindowPointer
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? dx = null,Object? dy = null,}) {
  return _then(WindowPointer_Scroll(
dx: null == dx ? _self.dx : dx // ignore: cast_nullable_to_non_nullable
as double,dy: null == dy ? _self.dy : dy // ignore: cast_nullable_to_non_nullable
as double,
  ));
}


}

/// @nodoc
mixin _$WindowShared {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is WindowShared);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'WindowShared()';
}


}

/// @nodoc
class $WindowSharedCopyWith<$Res>  {
$WindowSharedCopyWith(WindowShared _, $Res Function(WindowShared) __);
}


/// Adds pattern-matching-related methods to [WindowShared].
extension WindowSharedPatterns on WindowShared {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( WindowShared_NothingChosen value)?  nothingChosen,TResult Function( WindowShared_Started value)?  started,TResult Function( WindowShared_Declined value)?  declined,required TResult orElse(),}){
final _that = this;
switch (_that) {
case WindowShared_NothingChosen() when nothingChosen != null:
return nothingChosen(_that);case WindowShared_Started() when started != null:
return started(_that);case WindowShared_Declined() when declined != null:
return declined(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( WindowShared_NothingChosen value)  nothingChosen,required TResult Function( WindowShared_Started value)  started,required TResult Function( WindowShared_Declined value)  declined,}){
final _that = this;
switch (_that) {
case WindowShared_NothingChosen():
return nothingChosen(_that);case WindowShared_Started():
return started(_that);case WindowShared_Declined():
return declined(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( WindowShared_NothingChosen value)?  nothingChosen,TResult? Function( WindowShared_Started value)?  started,TResult? Function( WindowShared_Declined value)?  declined,}){
final _that = this;
switch (_that) {
case WindowShared_NothingChosen() when nothingChosen != null:
return nothingChosen(_that);case WindowShared_Started() when started != null:
return started(_that);case WindowShared_Declined() when declined != null:
return declined(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function()?  nothingChosen,TResult Function( WindowSession session)?  started,TResult Function( String message)?  declined,required TResult orElse(),}) {final _that = this;
switch (_that) {
case WindowShared_NothingChosen() when nothingChosen != null:
return nothingChosen();case WindowShared_Started() when started != null:
return started(_that.session);case WindowShared_Declined() when declined != null:
return declined(_that.message);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function()  nothingChosen,required TResult Function( WindowSession session)  started,required TResult Function( String message)  declined,}) {final _that = this;
switch (_that) {
case WindowShared_NothingChosen():
return nothingChosen();case WindowShared_Started():
return started(_that.session);case WindowShared_Declined():
return declined(_that.message);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function()?  nothingChosen,TResult? Function( WindowSession session)?  started,TResult? Function( String message)?  declined,}) {final _that = this;
switch (_that) {
case WindowShared_NothingChosen() when nothingChosen != null:
return nothingChosen();case WindowShared_Started() when started != null:
return started(_that.session);case WindowShared_Declined() when declined != null:
return declined(_that.message);case _:
  return null;

}
}

}

/// @nodoc


class WindowShared_NothingChosen extends WindowShared {
  const WindowShared_NothingChosen(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is WindowShared_NothingChosen);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'WindowShared.nothingChosen()';
}


}




/// @nodoc


class WindowShared_Started extends WindowShared {
  const WindowShared_Started({required this.session}): super._();
  

/// The session.
 final  WindowSession session;

/// Create a copy of WindowShared
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$WindowShared_StartedCopyWith<WindowShared_Started> get copyWith => _$WindowShared_StartedCopyWithImpl<WindowShared_Started>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is WindowShared_Started&&(identical(other.session, session) || other.session == session));
}


@override
int get hashCode {
    return Object.hash(runtimeType,session);
}

@override
String toString() {
    return 'WindowShared.started(session: $session)';
}


}

/// @nodoc
abstract mixin class $WindowShared_StartedCopyWith<$Res> implements $WindowSharedCopyWith<$Res> {
  factory $WindowShared_StartedCopyWith(WindowShared_Started value, $Res Function(WindowShared_Started) _then) = _$WindowShared_StartedCopyWithImpl;
@useResult
$Res call({
 WindowSession session
});




}
/// @nodoc
class _$WindowShared_StartedCopyWithImpl<$Res>
    implements $WindowShared_StartedCopyWith<$Res> {
  _$WindowShared_StartedCopyWithImpl(this._self, this._then);

  final WindowShared_Started _self;
  final $Res Function(WindowShared_Started) _then;

/// Create a copy of WindowShared
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? session = null,}) {
  return _then(WindowShared_Started(
session: null == session ? _self.session : session // ignore: cast_nullable_to_non_nullable
as WindowSession,
  ));
}


}

/// @nodoc


class WindowShared_Declined extends WindowShared {
  const WindowShared_Declined({required this.message}): super._();
  

/// One sentence that says why, in words that name the device.
 final  String message;

/// Create a copy of WindowShared
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$WindowShared_DeclinedCopyWith<WindowShared_Declined> get copyWith => _$WindowShared_DeclinedCopyWithImpl<WindowShared_Declined>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is WindowShared_Declined&&(identical(other.message, message) || other.message == message));
}


@override
int get hashCode {
    return Object.hash(runtimeType,message);
}

@override
String toString() {
    return 'WindowShared.declined(message: $message)';
}


}

/// @nodoc
abstract mixin class $WindowShared_DeclinedCopyWith<$Res> implements $WindowSharedCopyWith<$Res> {
  factory $WindowShared_DeclinedCopyWith(WindowShared_Declined value, $Res Function(WindowShared_Declined) _then) = _$WindowShared_DeclinedCopyWithImpl;
@useResult
$Res call({
 String message
});




}
/// @nodoc
class _$WindowShared_DeclinedCopyWithImpl<$Res>
    implements $WindowShared_DeclinedCopyWith<$Res> {
  _$WindowShared_DeclinedCopyWithImpl(this._self, this._then);

  final WindowShared_Declined _self;
  final $Res Function(WindowShared_Declined) _then;

/// Create a copy of WindowShared
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? message = null,}) {
  return _then(WindowShared_Declined(
message: null == message ? _self.message : message // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc
mixin _$WindowUnlock {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is WindowUnlock);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'WindowUnlock()';
}


}

/// @nodoc
class $WindowUnlockCopyWith<$Res>  {
$WindowUnlockCopyWith(WindowUnlock _, $Res Function(WindowUnlock) __);
}


/// Adds pattern-matching-related methods to [WindowUnlock].
extension WindowUnlockPatterns on WindowUnlock {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( WindowUnlock_Asking value)?  asking,TResult Function( WindowUnlock_Typing value)?  typing,TResult Function( WindowUnlock_Ended value)?  ended,required TResult orElse(),}){
final _that = this;
switch (_that) {
case WindowUnlock_Asking() when asking != null:
return asking(_that);case WindowUnlock_Typing() when typing != null:
return typing(_that);case WindowUnlock_Ended() when ended != null:
return ended(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( WindowUnlock_Asking value)  asking,required TResult Function( WindowUnlock_Typing value)  typing,required TResult Function( WindowUnlock_Ended value)  ended,}){
final _that = this;
switch (_that) {
case WindowUnlock_Asking():
return asking(_that);case WindowUnlock_Typing():
return typing(_that);case WindowUnlock_Ended():
return ended(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( WindowUnlock_Asking value)?  asking,TResult? Function( WindowUnlock_Typing value)?  typing,TResult? Function( WindowUnlock_Ended value)?  ended,}){
final _that = this;
switch (_that) {
case WindowUnlock_Asking() when asking != null:
return asking(_that);case WindowUnlock_Typing() when typing != null:
return typing(_that);case WindowUnlock_Ended() when ended != null:
return ended(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function()?  asking,TResult Function()?  typing,TResult Function( UnlockResult result)?  ended,required TResult orElse(),}) {final _that = this;
switch (_that) {
case WindowUnlock_Asking() when asking != null:
return asking();case WindowUnlock_Typing() when typing != null:
return typing();case WindowUnlock_Ended() when ended != null:
return ended(_that.result);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function()  asking,required TResult Function()  typing,required TResult Function( UnlockResult result)  ended,}) {final _that = this;
switch (_that) {
case WindowUnlock_Asking():
return asking();case WindowUnlock_Typing():
return typing();case WindowUnlock_Ended():
return ended(_that.result);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function()?  asking,TResult? Function()?  typing,TResult? Function( UnlockResult result)?  ended,}) {final _that = this;
switch (_that) {
case WindowUnlock_Asking() when asking != null:
return asking();case WindowUnlock_Typing() when typing != null:
return typing();case WindowUnlock_Ended() when ended != null:
return ended(_that.result);case _:
  return null;

}
}

}

/// @nodoc


class WindowUnlock_Asking extends WindowUnlock {
  const WindowUnlock_Asking(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is WindowUnlock_Asking);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'WindowUnlock.asking()';
}


}




/// @nodoc


class WindowUnlock_Typing extends WindowUnlock {
  const WindowUnlock_Typing(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is WindowUnlock_Typing);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'WindowUnlock.typing()';
}


}




/// @nodoc


class WindowUnlock_Ended extends WindowUnlock {
  const WindowUnlock_Ended({required this.result}): super._();
  

/// How it ended.
 final  UnlockResult result;

/// Create a copy of WindowUnlock
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$WindowUnlock_EndedCopyWith<WindowUnlock_Ended> get copyWith => _$WindowUnlock_EndedCopyWithImpl<WindowUnlock_Ended>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is WindowUnlock_Ended&&(identical(other.result, result) || other.result == result));
}


@override
int get hashCode {
    return Object.hash(runtimeType,result);
}

@override
String toString() {
    return 'WindowUnlock.ended(result: $result)';
}


}

/// @nodoc
abstract mixin class $WindowUnlock_EndedCopyWith<$Res> implements $WindowUnlockCopyWith<$Res> {
  factory $WindowUnlock_EndedCopyWith(WindowUnlock_Ended value, $Res Function(WindowUnlock_Ended) _then) = _$WindowUnlock_EndedCopyWithImpl;
@useResult
$Res call({
 UnlockResult result
});




}
/// @nodoc
class _$WindowUnlock_EndedCopyWithImpl<$Res>
    implements $WindowUnlock_EndedCopyWith<$Res> {
  _$WindowUnlock_EndedCopyWithImpl(this._self, this._then);

  final WindowUnlock_Ended _self;
  final $Res Function(WindowUnlock_Ended) _then;

/// Create a copy of WindowUnlock
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? result = null,}) {
  return _then(WindowUnlock_Ended(
result: null == result ? _self.result : result // ignore: cast_nullable_to_non_nullable
as UnlockResult,
  ));
}


}

// dart format on
