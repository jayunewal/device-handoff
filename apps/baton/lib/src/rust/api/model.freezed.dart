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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( Event_Received value)?  received,TResult Function( Event_DevicesChanged value)?  devicesChanged,TResult Function( Event_PairingRequested value)?  pairingRequested,TResult Function( Event_PairingRequestEnded value)?  pairingRequestEnded,TResult Function( Event_OfferEnded value)?  offerEnded,TResult Function( Event_TransferRequested value)?  transferRequested,TResult Function( Event_TransferRequestEnded value)?  transferRequestEnded,TResult Function( Event_TransferEnded value)?  transferEnded,TResult Function( Event_Resync value)?  resync,required TResult orElse(),}){
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
return transferEnded(_that);case Event_Resync() when resync != null:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( Event_Received value)  received,required TResult Function( Event_DevicesChanged value)  devicesChanged,required TResult Function( Event_PairingRequested value)  pairingRequested,required TResult Function( Event_PairingRequestEnded value)  pairingRequestEnded,required TResult Function( Event_OfferEnded value)  offerEnded,required TResult Function( Event_TransferRequested value)  transferRequested,required TResult Function( Event_TransferRequestEnded value)  transferRequestEnded,required TResult Function( Event_TransferEnded value)  transferEnded,required TResult Function( Event_Resync value)  resync,}){
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
return transferEnded(_that);case Event_Resync():
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( Event_Received value)?  received,TResult? Function( Event_DevicesChanged value)?  devicesChanged,TResult? Function( Event_PairingRequested value)?  pairingRequested,TResult? Function( Event_PairingRequestEnded value)?  pairingRequestEnded,TResult? Function( Event_OfferEnded value)?  offerEnded,TResult? Function( Event_TransferRequested value)?  transferRequested,TResult? Function( Event_TransferRequestEnded value)?  transferRequestEnded,TResult? Function( Event_TransferEnded value)?  transferEnded,TResult? Function( Event_Resync value)?  resync,}){
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
return transferEnded(_that);case Event_Resync() when resync != null:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( Received received)?  received,TResult Function()?  devicesChanged,TResult Function( PairingRequest request)?  pairingRequested,TResult Function( PlatformInt64 id)?  pairingRequestEnded,TResult Function( OfferOutcome outcome)?  offerEnded,TResult Function( TransferRequest request)?  transferRequested,TResult Function( PlatformInt64 id)?  transferRequestEnded,TResult Function( TransferEnd end)?  transferEnded,TResult Function()?  resync,required TResult orElse(),}) {final _that = this;
switch (_that) {
case Event_Received() when received != null:
return received(_that.received);case Event_DevicesChanged() when devicesChanged != null:
return devicesChanged();case Event_PairingRequested() when pairingRequested != null:
return pairingRequested(_that.request);case Event_PairingRequestEnded() when pairingRequestEnded != null:
return pairingRequestEnded(_that.id);case Event_OfferEnded() when offerEnded != null:
return offerEnded(_that.outcome);case Event_TransferRequested() when transferRequested != null:
return transferRequested(_that.request);case Event_TransferRequestEnded() when transferRequestEnded != null:
return transferRequestEnded(_that.id);case Event_TransferEnded() when transferEnded != null:
return transferEnded(_that.end);case Event_Resync() when resync != null:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( Received received)  received,required TResult Function()  devicesChanged,required TResult Function( PairingRequest request)  pairingRequested,required TResult Function( PlatformInt64 id)  pairingRequestEnded,required TResult Function( OfferOutcome outcome)  offerEnded,required TResult Function( TransferRequest request)  transferRequested,required TResult Function( PlatformInt64 id)  transferRequestEnded,required TResult Function( TransferEnd end)  transferEnded,required TResult Function()  resync,}) {final _that = this;
switch (_that) {
case Event_Received():
return received(_that.received);case Event_DevicesChanged():
return devicesChanged();case Event_PairingRequested():
return pairingRequested(_that.request);case Event_PairingRequestEnded():
return pairingRequestEnded(_that.id);case Event_OfferEnded():
return offerEnded(_that.outcome);case Event_TransferRequested():
return transferRequested(_that.request);case Event_TransferRequestEnded():
return transferRequestEnded(_that.id);case Event_TransferEnded():
return transferEnded(_that.end);case Event_Resync():
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( Received received)?  received,TResult? Function()?  devicesChanged,TResult? Function( PairingRequest request)?  pairingRequested,TResult? Function( PlatformInt64 id)?  pairingRequestEnded,TResult? Function( OfferOutcome outcome)?  offerEnded,TResult? Function( TransferRequest request)?  transferRequested,TResult? Function( PlatformInt64 id)?  transferRequestEnded,TResult? Function( TransferEnd end)?  transferEnded,TResult? Function()?  resync,}) {final _that = this;
switch (_that) {
case Event_Received() when received != null:
return received(_that.received);case Event_DevicesChanged() when devicesChanged != null:
return devicesChanged();case Event_PairingRequested() when pairingRequested != null:
return pairingRequested(_that.request);case Event_PairingRequestEnded() when pairingRequestEnded != null:
return pairingRequestEnded(_that.id);case Event_OfferEnded() when offerEnded != null:
return offerEnded(_that.outcome);case Event_TransferRequested() when transferRequested != null:
return transferRequested(_that.request);case Event_TransferRequestEnded() when transferRequestEnded != null:
return transferRequestEnded(_that.id);case Event_TransferEnded() when transferEnded != null:
return transferEnded(_that.end);case Event_Resync() when resync != null:
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




// dart format on
