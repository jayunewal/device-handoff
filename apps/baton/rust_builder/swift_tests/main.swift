// SPDX-License-Identifier: Apache-2.0

import Foundation

var tests: [(String, () -> Void)] = [
  ("registration gives the Rust side a whole table", testRegistrationGivesTheRustSideAWholeTable),
  (
    "a registration that fails returns its code and lets go of the platform",
    testARegistrationThatFailsReturnsItsCodeAndLetsGoOfThePlatform
  ),
  (
    "a registration that works keeps the platform for the life of the process",
    testARegistrationThatWorksKeepsThePlatformForTheLifeOfTheProcess
  ),
  (
    "limits are what the platform says and the room is checked",
    testLimitsAreWhatThePlatformSaysAndTheRoomIsChecked
  ),
  (
    "an open makes a screen and gives its pointer and number",
    testAnOpenMakesAScreenAndGivesItsPointerAndNumber
  ),
  (
    "opens that overlap each other and the calls of a screen are safe",
    testOpensThatOverlapEachOtherAndTheCallsOfAScreenAreSafe
  ),
  (
    "an open that fails writes the sentence and makes no pointer",
    testAnOpenThatFailsWritesTheSentenceAndMakesNoPointer
  ),
  (
    "an open that fails in another way has a sentence of its own",
    testAnOpenThatFailsInAnotherWayHasASentenceOfItsOwn
  ),
  (
    "a sentence that does not fit is cut between two characters",
    testASentenceThatDoesNotFitIsCutBetweenTwoCharacters
  ),
  (
    "an open with structs that are too short or missing is refused",
    testAnOpenWithStructsThatAreTooShortOrMissingIsRefused
  ),
  ("a message with no room writes nothing", testAMessageWithNoRoomWritesNothing),
  ("a frame reaches the screen as the Rust side built it", testAFrameReachesTheScreenAsTheRustSideBuiltIt),
  (
    "a frame that is not well formed never reaches the screen",
    testAFrameThatIsNotWellFormedNeverReachesTheScreen
  ),
  ("what the source says reaches the screen", testWhatTheSourceSaysReachesTheScreen),
  ("show, state and ended reach the screen", testShowStateAndEndedReachTheScreen),
  (
    "release tells the screen once and lets go of it",
    testReleaseTellsTheScreenOnceAndLetsGoOfIt
  ),
  (
    "a picture is shown when both the decoder and the Rust side have said so",
    testAPictureIsShownWhenBothTheDecoderAndTheRustSideHaveSaidSo
  ),
  (
    "an older picture is never shown after a newer one",
    testAnOlderPictureIsNeverShownAfterANewerOne
  ),
  (
    "a frame that never came out does not hold back the next",
    testAFrameThatNeverCameOutDoesNotHoldBackTheNext
  ),
  (
    "pictures that nobody asks for are kept up to the capacity",
    testPicturesThatNobodyAsksForAreKeptUpToTheCapacity
  ),
  (
    "what is older than the picture shown is let go of",
    testWhatIsOlderThanThePictureShownIsLetGoOf
  ),
  ("clear forgets everything", testClearForgetsEverythingIncludingWhatWasShown),
  (
    "the time a frame takes is smoothed and never out of range",
    testTheTimeAFrameTakesIsSmoothedAndNeverOutOfRange
  ),
  (
    "a key frame prepares the decoder and later frames do not",
    testAKeyFramePreparesTheDecoderAndLaterFramesDoNot
  ),
  ("new parameter sets prepare the decoder again", testNewParameterSetsPrepareTheDecoderAgain),
  (
    "a frame is not decoded before a key frame has come",
    testAFrameIsNotDecodedBeforeAKeyFrameHasCome
  ),
  (
    "a key frame that came with no parameter sets is refused",
    testAKeyFrameThatCameWithNoParameterSetsIsRefused
  ),
  (
    "parameter sets that the platform cannot use refuse the frame",
    testParameterSetsThatThePlatformCannotUseRefuseTheFrame
  ),
  (
    "after parameters that were refused nothing is decoded until a key frame comes",
    testAfterParametersThatWereRefusedNothingIsDecodedUntilAKeyFrameComes
  ),
  (
    "a frame that the decoder does not take makes the screen wait for a key frame",
    testAFrameThatTheDecoderDoesNotTakeMakesTheScreenWaitForAKeyFrame
  ),
  (
    "a picture is shown in either order of the decoder and the Rust side",
    testAPictureIsShownInEitherOrderOfTheDecoderAndTheRustSide
  ),
  (
    "a late failure is reported once and a key frame starts again",
    testALateFailureIsReportedOnceAndAKeyFrameStartsAgain
  ),
  (
    "a failure is not reported from the thread that is inside the call",
    testAFailureIsNotReportedFromTheThreadThatIsInsideTheCall
  ),
  (
    "nothing is shown or reported after the session ended",
    testNothingIsShownOrReportedAfterTheSessionEnded
  ),
  (
    "a picture on its way to the screen is there before the end takes it away",
    testAPictureOnItsWayToTheScreenIsThereBeforeTheEndTakesItAway
  ),
  (
    "pictures reach the screen one at a time in the order they were chosen",
    testPicturesReachTheScreenOneAtATimeInTheOrderTheyWereChosen
  ),
  (
    "a picture that comes after the end is not kept",
    testAPictureThatComesAfterTheEndIsNotKept
  ),
  (
    "releasing a screen ends it and lets go of what it held once",
    testReleasingAScreenEndsItAndLetsGoOfWhatItHeldOnce
  ),
  (
    "the state tells how long frames take and how hot the device is",
    testTheStateTellsHowLongFramesTakeAndHowHotTheDeviceIs
  ),
  ("a frame is given the sample and nothing of the sets", testAFrameIsGivenTheSampleAndNothingOfTheSets),
  (
    "parameter sets are copies split at the sequence sets",
    testParameterSetsAreCopiesSplitAtTheSequenceSets
  ),
  (
    "a failure that comes while a frame is prepared for is not undone",
    testAFailureThatComesWhileAFrameIsPreparedForIsNotUndone
  ),
  (
    "a screen that ends while a frame is prepared for decodes nothing",
    testAScreenThatEndsWhileAFrameIsPreparedForDecodesNothing
  ),
  (
    "a picture that was held is not shown after the end",
    testAPictureThatWasHeldIsNotShownAfterTheEnd
  ),
  ("only the key flag makes a key frame", testOnlyTheKeyFlagMakesAKeyFrame),
  (
    "the state can be read while pictures come out",
    testTheStateCanBeReadWhilePicturesComeOut
  ),
  (
    "places are made when asked, and only the missing ones",
    testPlacesAreMadeWhenAskedAndOnlyTheMissingOnesAre
  ),
  (
    "a place that cannot be made stops the filling and is tried again",
    testAPlaceThatCannotBeMadeStopsTheFillingAndIsTriedAgain
  ),
  ("a place is used by one window at a time", testAPlaceIsUsedByOneWindowAtATime),
  ("no place is taken before any is made", testNoPlaceIsTakenBeforeAnyIsMade),
  ("a count below one wants nothing", testACountBelowOneWantsNothing),
  (
    "fills at the same time make each place once",
    testFillsAtTheSameTimeMakeEachPlaceOnce
  ),
  (
    "a place can be taken while the others are still being made",
    testAPlaceCanBeTakenWhileTheOthersAreStillBeingMade
  ),
  (
    "places are taken and given from many threads without losing one",
    testPlacesAreTakenAndGivenFromManyThreadsWithoutLosingOne
  ),
  ("an attempt that works is not repeated", testAnAttemptThatWorksIsNotRepeated),
  (
    "an attempt that does not work is repeated after each delay and then given up",
    testAnAttemptThatDoesNotWorkIsRepeatedAfterEachDelayAndThenGivenUp
  ),
  ("trying stops as soon as an attempt works", testTryingStopsAsSoonAsAnAttemptWorks),
  ("no delays means one attempt", testNoDelaysMeansOneAttempt),
  (
    "places are made once the engine runs, as on a Mac",
    testPlacesAreMadeOnceTheEngineRunsAsTheMacDoes
  ),
  (
    "places that never come are given up on, and made when a window asks",
    testPlacesThatNeverComeAreGivenUpOnAndAskedForAgainWhenAWindowComes
  ),
]

#if canImport(VideoToolbox)
  tests += [
    (
      "VideoToolbox gives back the samples that were coded",
      testVideoToolboxGivesBackTheSamplesThatWereCoded
    ),
    (
      "VideoToolbox goes on when the same parameter sets come again",
      testVideoToolboxGoesOnWhenTheSameParameterSetsComeAgain
    ),
    ("VideoToolbox starts again for another size", testVideoToolboxStartsAgainForAnotherSize),
    (
      "VideoToolbox makes the format that Flutter draws",
      testVideoToolboxMakesTheFormatThatFlutterDraws
    ),
    (
      "VideoToolbox answers every frame that it took",
      testVideoToolboxAnswersEveryFrameThatItTook
    ),
    ("VideoToolbox refuses what it cannot use", testVideoToolboxRefusesWhatItCannotUse),
    (
      "VideoToolbox pictures reach the screen through the whole chain",
      testVideoToolboxPicturesReachTheScreenThroughTheWholeChain
    ),
    (
      "a picture with nothing in it is one that Flutter draws",
      testAPictureWithNothingInItIsOneThatFlutterDraws
    ),
  ]
#endif

for (name, body) in tests {
  let before = failures
  body()
  print("\(failures == before ? "ok  " : "FAIL") \(name)")
}
print("\(tests.count) tests, \(checks) checks, \(failures) failures")
exit(failures == 0 ? 0 : 1)
